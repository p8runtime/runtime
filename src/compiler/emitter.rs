use crate::compiler::chunk::{CatchEntry, CodeChunk, CompileError, ConstExpr, FuncParam, ReturnType, UserFunc};
use crate::core::interner::Interner;
use crate::core::value::{Symbol, Val, Visibility};
use crate::parser::ast::{
    Arg, AssignOp, AttributeGroup, BinaryOp, CastKind, ClassMember, Expr, IncludeKind, MagicConstKind,
    Stmt, StmtId, Type, UnaryOp,
};
use crate::parser::lexer::token::{Token, TokenKind};
use crate::runtime::context::{AttributeArg, AttributeInstance};
use crate::vm::engine::ErrorLevel;
use crate::vm::opcode::OpCode;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

/// Unescape a double-quoted string (or heredoc / interpolated part) the way
/// Zend's scanner does: \n \t \r \v \e \f \\ \$ \", octal \[0-7]{1,3},
/// hex \x[0-9a-f]{1,2} and \u{codepoint}; any other backslash is kept.
/// Reference: $PHP_SRC_PATH/Zend/zend_language_scanner.l - zend_scan_escape_string
pub(crate) fn unescape_string(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] != b'\\' || i + 1 >= s.len() {
            out.push(s[i]);
            i += 1;
            continue;
        }
        let c = s[i + 1];
        i += 2;
        match c {
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'v' => out.push(0x0B),
            b'e' => out.push(0x1B),
            b'f' => out.push(0x0C),
            b'\\' | b'$' | b'"' => out.push(c),
            b'0'..=b'7' => {
                let mut value = u32::from(c - b'0');
                let mut digits = 1;
                while digits < 3 && i < s.len() && (b'0'..=b'7').contains(&s[i]) {
                    value = value * 8 + u32::from(s[i] - b'0');
                    i += 1;
                    digits += 1;
                }
                out.push(value as u8); // "\400" wraps, as in PHP
            }
            b'x' if i < s.len() && s[i].is_ascii_hexdigit() => {
                let mut value = 0u32;
                let mut digits = 0;
                while digits < 2 && i < s.len() && s[i].is_ascii_hexdigit() {
                    value = value * 16 + char::from(s[i]).to_digit(16).unwrap_or(0);
                    i += 1;
                    digits += 1;
                }
                out.push(value as u8);
            }
            b'u' if s.get(i) == Some(&b'{') => {
                let end = s[i..].iter().position(|b| *b == b'}').map(|p| i + p);
                let code = end.and_then(|end| {
                    std::str::from_utf8(&s[i + 1..end])
                        .ok()
                        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                        .and_then(char::from_u32)
                });
                match (end, code) {
                    (Some(end), Some(ch)) => {
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                        i = end + 1;
                    }
                    _ => out.extend_from_slice(b"\\u"),
                }
            }
            _ => {
                out.push(b'\\');
                out.push(c);
            }
        }
    }
    out
}

/// The value of an integer literal: decimal, 0x hex, 0b binary, 0o or
/// leading-0 octal, with `_` separators; too large for an int, it is a float.
/// Reference: $PHP_SRC_PATH/Zend/zend_language_scanner.l - LNUM/HNUM/BNUM/ONUM
pub(crate) fn integer_literal(value: &[u8]) -> Val {
    let digits: Vec<u8> = value.iter().copied().filter(|b| *b != b'_').collect();
    let (radix, body) = match digits.as_slice() {
        [b'0', b'x' | b'X', rest @ ..] => (16, rest),
        [b'0', b'b' | b'B', rest @ ..] => (2, rest),
        [b'0', b'o' | b'O', rest @ ..] => (8, rest),
        [b'0', rest @ ..] if !rest.is_empty() => (8, rest),
        all => (10, all),
    };
    let text = std::str::from_utf8(body).unwrap_or("0");
    match i64::from_str_radix(text, radix) {
        Ok(i) => Val::Int(i),
        // Too large for an int: a float, correctly rounded for decimals
        Err(_) if radix == 10 => Val::Float(text.parse().unwrap_or(0.0)),
        Err(_) => {
            let f = text
                .bytes()
                .filter_map(|b| char::from(b).to_digit(radix))
                .fold(0.0, |acc, d| acc * f64::from(radix) + f64::from(d));
            Val::Float(f)
        }
    }
}

/// The value of a float literal (`_` separators allowed).
pub(crate) fn float_literal(value: &[u8]) -> f64 {
    let text: String = value.iter().filter(|b| **b != b'_').map(|b| char::from(*b)).collect();
    text.parse().unwrap_or(0.0)
}

/// The value of a string literal token: '...' (only \\ and \' are escapes),
/// "..." (full escapes) or an unquoted interpolation/heredoc part.
pub(crate) fn string_literal(value: &[u8]) -> Vec<u8> {
    let quoted = |q: u8| value.len() >= 2 && value[0] == q && value[value.len() - 1] == q;
    if quoted(b'\'') {
        let inner = &value[1..value.len() - 1];
        let mut out = Vec::with_capacity(inner.len());
        let mut i = 0;
        while i < inner.len() {
            if inner[i] == b'\\' && matches!(inner.get(i + 1), Some(b'\'' | b'\\')) {
                out.push(inner[i + 1]);
                i += 2;
            } else {
                out.push(inner[i]);
                i += 1;
            }
        }
        out
    } else if quoted(b'"') {
        unescape_string(&value[1..value.len() - 1])
    } else {
        unescape_string(value)
    }
}

struct LoopInfo {
    break_jumps: Vec<usize>,
    continue_jumps: Vec<usize>,
    /// Operand-stack values the loop keeps while its body runs (foreach: iterable and
    /// position; switch: the subject).
    stack_slots: u32,
    /// Whether the code at the loop's end pops those slots (a switch does;
    /// a foreach's exhausted iterator pops them itself, so `break` must).
    end_pops: bool,
}

#[derive(Clone)]
struct TryFinallyInfo {
    /// Index in catch_table for the finally-only entry
    catch_table_idx: usize,
    /// Start of the finally block code
    finally_start: u32,
    /// End of the finally block code (exclusive)
    finally_end: u32,
}

pub struct Emitter<'src> {
    chunk: CodeChunk,
    source: &'src [u8],
    interner: &'src mut Interner,
    loop_stack: Vec<LoopInfo>,
    try_finally_stack: Vec<TryFinallyInfo>,
    is_generator: bool,
    // Context for magic constants
    file_path: Option<String>,
    current_class: Option<Symbol>,
    current_trait: Option<Symbol>,
    current_function: Option<Symbol>,
    current_namespace: Option<Symbol>,
    // For eval(): inherit strict_types from parent scope if not explicitly declared
    inherited_strict_types: Option<bool>,
    // Counter for anonymous classes
    anonymous_class_counter: usize,
    /// Set while emitting the object of a property write, where `$a[]`
    /// fetches a new element for writing instead of being read.
    append_fetch_ok: bool,
    /// Open short-circuit chains: the `?->` jumps to patch at each end.
    nullsafe_jumps: Vec<Vec<usize>>,
    /// Source line of the statement or expression being emitted; every opcode
    /// is tagged with it (CodeChunk::lines), for errors, exceptions and traces.
    current_line: u32,
    /// Byte offsets where each source line starts, shared with child emitters.
    line_starts: Option<Rc<Vec<usize>>>,
    /// The file's first compile error, shared with child emitters.
    compile_error: Rc<RefCell<Option<CompileError>>>,
    /// The file's compile-time warnings, shared with child emitters.
    compile_warnings: Rc<RefCell<Vec<(ErrorLevel, CompileError)>>>,
    /// `use` imports of the current namespace, shared with child emitters.
    imports: Rc<Imports>,
    /// Start offsets of the top-level declarations already emitted by
    /// hoisting; the in-order pass skips them.
    hoisted: std::collections::HashSet<usize>,
}

/// Type bits for the magic method checks (zend_type MAY_BE_*).
mod magic_types {
    pub const NULL: u32 = 1 << 0;
    pub const FALSE: u32 = 1 << 1;
    pub const TRUE: u32 = 1 << 2;
    pub const BOOL: u32 = FALSE | TRUE;
    pub const INT: u32 = 1 << 3;
    pub const FLOAT: u32 = 1 << 4;
    pub const STRING: u32 = 1 << 5;
    pub const ARRAY: u32 = 1 << 6;
    pub const OBJECT: u32 = 1 << 7;
    pub const CALLABLE: u32 = 1 << 8;
    pub const VOID: u32 = 1 << 9;
    pub const STATIC: u32 = 1 << 10;
    pub const NEVER: u32 = 1 << 11;
    pub const ANY: u32 = NULL | BOOL | INT | FLOAT | STRING | ARRAY | OBJECT;

    /// zend_type_to_string of the masks the checks name.
    pub fn mask_name(mask: u32) -> &'static str {
        match mask {
            STRING => "string",
            ARRAY => "array",
            BOOL => "bool",
            VOID => "void",
            OBJECT => "object",
            m if m == ARRAY | NULL => "?array",
            _ => "mixed",
        }
    }
}

/// The names a namespace imports with `use` (zend_compile.c FC(imports),
/// FC(imports_function), FC(imports_const)). Class and function aliases
/// are case-insensitive (keys are lowercase); constant aliases are not.
#[derive(Debug, Clone, Default)]
struct Imports {
    classes: HashMap<Vec<u8>, Vec<u8>>,
    functions: HashMap<Vec<u8>, Vec<u8>>,
    constants: HashMap<Vec<u8>, Vec<u8>>,
}

/// A function or constant name after resolution: unqualified names inside
/// a namespace try the namespaced name first, then the global one, at run
/// time.
enum ResolvedName {
    Exact(Vec<u8>),
    Fallback { namespaced: Vec<u8>, global: Vec<u8> },
}

impl<'src> Emitter<'src> {
    pub fn new(source: &'src [u8], interner: &'src mut Interner) -> Self {
        Self {
            chunk: CodeChunk::default(),
            source,
            interner,
            loop_stack: Vec::new(),
            try_finally_stack: Vec::new(),
            is_generator: false,
            file_path: None,
            current_class: None,
            current_trait: None,
            current_function: None,
            current_namespace: None,
            inherited_strict_types: None,
            anonymous_class_counter: 0,
            append_fetch_ok: false,
            nullsafe_jumps: Vec::new(),
            current_line: 1,
            line_starts: None,
            compile_error: Rc::default(),
            compile_warnings: Rc::default(),
            imports: Rc::default(),
            hoisted: Default::default(),
        }
    }

    /// The source text of a name, keeping a leading `\\` (fully qualified).
    fn raw_text(&self, span: crate::parser::span::Span) -> &'src [u8] {
        &self.source[span.start..span.end]
    }

    /// The current namespace, without a trailing separator ("" if global).
    fn namespace_name(&self) -> Vec<u8> {
        self.current_namespace
            .and_then(|ns| self.interner.lookup(ns))
            .unwrap_or(b"")
            .to_vec()
    }

    /// `name` inside the current namespace (a declaration's full name).
    fn prefix_namespace(&self, name: &[u8]) -> Vec<u8> {
        let ns = self.namespace_name();
        if ns.is_empty() {
            return name.to_vec();
        }
        [ns.as_slice(), b"\\", name].concat()
    }

    /// A qualified name whose first segment may be an imported alias:
    /// `A\B` with `use X\Y as A` is `X\Y\B`; otherwise it is namespaced.
    fn resolve_qualified(&self, name: &[u8]) -> Vec<u8> {
        let (first, rest) = match name.iter().position(|&b| b == b'\\') {
            Some(i) => (&name[..i], &name[i..]),
            None => (name, &b""[..]),
        };
        match self.imports.classes.get(&first.to_ascii_lowercase()) {
            Some(full) => [full.as_slice(), rest].concat(),
            None => self.prefix_namespace(name),
        }
    }

    /// A class name as written, resolved like zend_resolve_class_name: fully
    /// qualified names lose the `\\`, `namespace\\X` is relative to the
    /// current namespace, imports apply to the first segment, and
    /// everything else is in the current namespace. self, parent, static
    /// and the reserved type names stay as they are.
    fn resolve_class_name(&self, name: &[u8]) -> Vec<u8> {
        if name.starts_with(b"$") {
            return name.to_vec();
        }
        if let Some(rest) = name.strip_prefix(b"\\") {
            return rest.to_vec();
        }
        if let Some(rest) = Self::strip_namespace_keyword(name) {
            return self.prefix_namespace(rest);
        }
        if !name.contains(&b'\\') && Self::is_reserved_class_name(name) {
            return name.to_vec();
        }
        self.resolve_qualified(name)
    }

    /// The resolved class name of a span, interned.
    fn class_sym(&mut self, span: crate::parser::span::Span) -> Symbol {
        let name = self.resolve_class_name(self.raw_text(span));
        self.interner.intern(&name)
    }

    /// `namespace\\X` (case-insensitive keyword) → `X`.
    fn strip_namespace_keyword(name: &[u8]) -> Option<&[u8]> {
        (name.len() > 10 && name[..10].eq_ignore_ascii_case(b"namespace\\")).then(|| &name[10..])
    }

    /// Names that are never resolved as classes (zend_is_reserved_class_name
    /// plus the class fetch keywords).
    fn is_reserved_class_name(name: &[u8]) -> bool {
        const RESERVED: &[&[u8]] = &[
            b"bool", b"false", b"float", b"int", b"null", b"parent", b"self", b"static",
            b"string", b"true", b"void", b"never", b"iterable", b"object", b"mixed",
            b"array", b"callable",
        ];
        RESERVED.iter().any(|r| r.eq_ignore_ascii_case(name))
    }

    /// A function (`constant` false) or constant name as written, resolved
    /// like zend_resolve_function_name / zend_resolve_const_name.
    fn resolve_function_or_constant(&self, name: &[u8], constant: bool) -> ResolvedName {
        if let Some(rest) = name.strip_prefix(b"\\") {
            return ResolvedName::Exact(rest.to_vec());
        }
        if let Some(rest) = Self::strip_namespace_keyword(name) {
            return ResolvedName::Exact(self.prefix_namespace(rest));
        }
        if name.contains(&b'\\') {
            return ResolvedName::Exact(self.resolve_qualified(name));
        }
        let imported = match constant {
            true => self.imports.constants.get(name),
            false => self.imports.functions.get(&name.to_ascii_lowercase()),
        };
        if let Some(full) = imported {
            return ResolvedName::Exact(full.clone());
        }
        // true, false and null are never namespaced
        let special = constant
            && [&b"true"[..], b"false", b"null"].iter().any(|n| n.eq_ignore_ascii_case(name));
        if self.namespace_name().is_empty() || special {
            return ResolvedName::Exact(name.to_vec());
        }
        ResolvedName::Fallback {
            namespaced: self.prefix_namespace(name),
            global: name.to_vec(),
        }
    }

    /// `namespace X;` / `namespace X { ... }`: a new namespace starts with
    /// no imports.
    fn enter_namespace(&mut self, name: Option<&crate::parser::ast::Name>) {
        self.current_namespace = name.map(|n| {
            let text = self.name_parts_text(n);
            self.interner.intern(&text)
        });
        self.imports = Rc::default();
    }

    /// A Name's text from its tokens (group use builds names from parts).
    fn name_parts_text(&self, name: &crate::parser::ast::Name) -> Vec<u8> {
        let mut text = Vec::new();
        for part in name.parts {
            text.extend_from_slice(&self.source[part.span.start..part.span.end]);
        }
        text
    }

    /// `use A\\B [as C]`, `use function ...`, `use const ...`, with
    /// zend_compile_use's checks.
    /// `report` is false in the hoisting pass, which sees every `use`
    /// before the in-order pass does.
    fn add_imports(
        &mut self,
        uses: &[crate::parser::ast::UseItem],
        kind: crate::parser::ast::UseKind,
        report: bool,
    ) {
        use crate::parser::ast::UseKind;
        for item in uses {
            let mut full = self.name_parts_text(&item.name);
            if full.first() == Some(&b'\\') {
                full.remove(0);
            }
            let compound = full.contains(&b'\\');
            let alias = match item.alias {
                Some(alias) => self.raw_text(alias.span).to_vec(),
                None => full.rsplit(|&b| b == b'\\').next().unwrap_or(&full).to_vec(),
            };
            // A group use item carries its own kind (`use A\{function f}`).
            let kind = if item.kind != UseKind::Normal { item.kind } else { kind };
            if report && item.alias.is_none() && !compound && self.namespace_name().is_empty() {
                self.compile_warning(
                    ErrorLevel::Warning,
                    item.span.start,
                    format!(
                        "The use statement with non-compound name '{}' has no effect",
                        String::from_utf8_lossy(&alias)
                    ),
                );
            }
            if report && kind == UseKind::Normal && Self::is_reserved_class_name(&alias) {
                self.compile_error(
                    item.span.start,
                    format!(
                        "Cannot use {} as {} because '{}' is a special class name",
                        String::from_utf8_lossy(&full),
                        String::from_utf8_lossy(&alias),
                        String::from_utf8_lossy(&alias)
                    ),
                );
            }
            let (key, type_str) = match kind {
                UseKind::Normal => (alias.to_ascii_lowercase(), ""),
                UseKind::Function => (alias.to_ascii_lowercase(), " function"),
                UseKind::Const => (alias.clone(), " const"),
            };
            let in_use = match kind {
                UseKind::Normal => self.imports.classes.contains_key(&key),
                UseKind::Function => self.imports.functions.contains_key(&key),
                UseKind::Const => self.imports.constants.contains_key(&key),
            };
            if in_use {
                if !report {
                    continue;
                }
                self.compile_error(
                    item.span.start,
                    format!(
                        "Cannot use{} {} as {} because the name is already in use",
                        type_str,
                        String::from_utf8_lossy(&full),
                        String::from_utf8_lossy(&alias)
                    ),
                );
                continue;
            }
            let imports = Rc::make_mut(&mut self.imports);
            match kind {
                UseKind::Normal => imports.classes.insert(key, full),
                UseKind::Function => imports.functions.insert(key, full),
                UseKind::Const => imports.constants.insert(key, full),
            };
        }
    }

    /// Record a compile-time error at `offset`; the first one is reported
    /// and the file does not run (zend_error_noreturn(E_COMPILE_ERROR)).
    fn compile_error(&mut self, offset: usize, message: impl Into<String>) {
        let line = self.line_at(offset);
        let mut slot = self.compile_error.borrow_mut();
        if slot.is_none() {
            *slot = Some(CompileError {
                message: message.into(),
                line,
            });
        }
    }

    /// A compile-time warning or deprecation at `offset` (E_COMPILE_WARNING
    /// and friends): shown when the file is loaded, before it runs.
    fn compile_warning(&mut self, level: ErrorLevel, offset: usize, message: impl Into<String>) {
        let line = self.line_at(offset);
        self.compile_warnings.borrow_mut().push((
            level,
            CompileError {
                message: message.into(),
                line,
            },
        ));
    }

    /// The 1-based source line of a byte offset.
    fn line_at(&mut self, offset: usize) -> u32 {
        let starts = self
            .line_starts
            .get_or_insert_with(|| {
                let mut starts = vec![0];
                starts.extend(
                    self.source
                        .iter()
                        .enumerate()
                        .filter(|(_, b)| **b == b'\n')
                        .map(|(i, _)| i + 1),
                );
                Rc::new(starts)
            })
            .clone();
        starts.partition_point(|start| *start <= offset) as u32
    }

    /// Before `break`/`continue` to `loop_stack[target]`: pop what the loops
    /// and switches being left keep on the operand stack. A `continue`
    /// keeps the target's own slots (it goes on iterating), except a
    /// switch's, whose end pops them; a `break` drops them unless the
    /// target's end pops them.
    fn emit_loop_exit_pops(&mut self, target: usize, is_break: bool) {
        let inner: u32 = self.loop_stack[target + 1..].iter().map(|l| l.stack_slots).sum();
        let own = &self.loop_stack[target];
        let own_pops = match (is_break, own.end_pops) {
            (_, true) => 0,
            (true, false) => own.stack_slots,
            (false, false) => 0,
        };
        for _ in 0..inner + own_pops {
            self.chunk.code.push(OpCode::Pop);
        }
    }

    /// Tag every opcode emitted since the last call with the current line.
    fn sync_lines(&mut self) {
        let missing = self.chunk.code.len().saturating_sub(self.chunk.lines.len());
        self.chunk
            .lines
            .extend(std::iter::repeat_n(self.current_line, missing));
    }

    /// Emit with `offset`'s line as the current line, restoring it after.
    fn at_line(&mut self, offset: usize, emit: impl FnOnce(&mut Self)) {
        self.sync_lines();
        let saved = self.current_line;
        self.current_line = self.line_at(offset);
        emit(self);
        self.sync_lines();
        self.current_line = saved;
    }

    /// Line state for a child emitter (a function or method body) starting at `offset`.
    fn child_lines(&mut self, offset: usize) -> (u32, Option<Rc<Vec<usize>>>) {
        (self.line_at(offset), self.line_starts.clone())
    }

    /// Create an emitter with a file path for accurate __FILE__ and __DIR__
    pub fn with_file_path(mut self, path: impl Into<String>) -> Self {
        self.file_path = Some(path.into());
        self
    }

    /// Set inherited strict_types for eval() - applies only if no explicit declare
    pub fn with_inherited_strict_types(mut self, strict: bool) -> Self {
        self.inherited_strict_types = Some(strict);
        self
    }

    fn get_visibility(&self, modifiers: &[Token]) -> Visibility {
        for token in modifiers {
            match token.kind {
                TokenKind::Public => return Visibility::Public,
                TokenKind::Protected => return Visibility::Protected,
                TokenKind::Private => return Visibility::Private,
                _ => {}
            }
        }
        Visibility::Public // Default
    }

    pub fn compile(mut self, stmts: &[StmtId]) -> (CodeChunk, bool) {
        // Check if any statement is an explicit declare(strict_types=...)
        let has_explicit_strict_types = stmts.iter().any(|stmt| {
            matches!(stmt, Stmt::Declare { declares, .. } if declares.iter().any(|item| {
                let key = self.get_text(item.key.span);
                key.eq_ignore_ascii_case(b"strict_types")
            }))
        });

        // Apply inherited strictness only if no explicit declare
        if !has_explicit_strict_types {
            if let Some(inherited) = self.inherited_strict_types {
                self.chunk.strict_types = inherited;
            }
        }

        // Only a file's own top level is hoisted, not a function body.
        if self.current_function.is_none() {
            self.hoist_declarations(stmts);
            self.enter_namespace(None);
        }
        for stmt in stmts {
            self.emit_stmt(stmt);
        }
        self.finish()
    }

    /// PHP declares a file's top-level functions, and the classes it can
    /// bind early, when the file is compiled: they exist before the first
    /// statement runs (zend_compile_func_decl, zend_compile_class_decl).
    /// Emit those declarations first. Early binding here covers classes
    /// with no parent, interfaces or traits; a class with a parent waits
    /// for its statement, as PHP does when the parent's or the methods'
    /// types are not all known yet.
    fn hoist_declarations(&mut self, stmts: &[StmtId]) {
        for stmt in stmts {
            match stmt {
                Stmt::Namespace { name, body, .. } => {
                    self.enter_namespace(name.as_ref());
                    if let Some(body) = body {
                        self.hoist_declarations(body);
                        self.enter_namespace(None);
                    }
                }
                Stmt::Use { uses, kind, .. } => self.add_imports(uses, *kind, false),
                Stmt::Function { span, .. } => {
                    self.emit_stmt(stmt);
                    self.hoisted.insert(span.start);
                }
                Stmt::Class {
                    extends: None,
                    implements: [],
                    members,
                    span,
                    ..
                } if !members.iter().any(|m| matches!(m, ClassMember::TraitUse { .. })) => {
                    self.emit_stmt(stmt);
                    self.hoisted.insert(span.start);
                }
                _ => {}
            }
        }
    }


    /// Compile an arrow function's body: `return <expr>;`.
    fn compile_arrow_body(mut self, expr: &Expr) -> (CodeChunk, bool) {
        if let Some(inherited) = self.inherited_strict_types {
            self.chunk.strict_types = inherited;
        }
        self.emit_expr(expr);
        self.chunk.code.push(OpCode::Return);
        self.finish()
    }

    fn finish(mut self) -> (CodeChunk, bool) {
        // Implicit return:
        // - Functions/methods: return null if no explicit return
        // - Top-level scripts: NO implicit return (PHP returns 1 for include, or the last statement result)
        if self.current_function.is_some() {
            // Inside a function - add implicit return null
            let null_idx = self.add_constant(Val::Null);
            self.chunk.code.push(OpCode::Const(null_idx as u16));
            self.chunk.code.push(OpCode::Return);
        }
        // Note: Top-level scripts don't get implicit return null

        self.sync_lines();

        let chunk_name = if let Some(func_sym) = self.current_function {
            func_sym
        } else if let Some(path) = &self.file_path {
            self.interner.intern(path.as_bytes())
        } else {
            self.interner.intern(b"(unknown)")
        };
        self.chunk.name = chunk_name;
        self.chunk.file_path = self.file_path.clone();
        // Child emitters (function bodies) are gone by the time the file's
        // own emitter finishes: only it holds the error then, and reports it.
        if Rc::strong_count(&self.compile_error) == 1 {
            self.chunk.compile_error = self.compile_error.borrow_mut().take();
            self.chunk.compile_warnings = std::mem::take(&mut *self.compile_warnings.borrow_mut());
        }

        (self.chunk, self.is_generator)
    }

    fn emit_members(&mut self, class_sym: Symbol, members: &[ClassMember]) {
        for member in members {
            match member {
                ClassMember::Method {
                    name,
                    body,
                    params,
                    modifiers,
                    return_type,
                    attributes,
                    by_ref,
                    ..
                } => {
                    let method_name_str = self.get_text(name.span);
                    let method_sym = self.interner.intern(method_name_str);
                    let visibility = self.get_visibility(modifiers);
                    let is_static = modifiers.iter().any(|t| t.kind == TokenKind::Static);
                    let is_abstract = modifiers.iter().any(|t| t.kind == TokenKind::Abstract);
                    self.check_magic_method(class_sym, name, params, is_static, visibility, *return_type);

                    // 1. Collect param info
                    struct ParamInfo<'a> {
                        name_span: crate::parser::span::Span,
                        by_ref: bool,
                        ty: Option<&'a Type<'a>>,
                        default: Option<&'a Expr<'a>>,
                        variadic: bool,
                        attributes: &'a [AttributeGroup<'a>],
                    }

                    let mut param_infos = Vec::new();
                    for param in *params {
                        param_infos.push(ParamInfo {
                            name_span: param.name.span,
                            by_ref: param.by_ref,
                            ty: param.ty,
                            default: param.default.as_ref().map(|e| *e),
                            variadic: param.variadic,
                            attributes: param.attributes,
                        });
                    }

                    // 2. Create emitter with inherited context
                    let (child_line, child_line_starts) = self.child_lines(name.span.start);
                    let compile_error = self.compile_error.clone();
                    let mut method_emitter = Emitter::new(self.source, self.interner);
                    method_emitter.compile_error = compile_error;
                    method_emitter.compile_warnings = self.compile_warnings.clone();
                    method_emitter.imports = self.imports.clone();
                    method_emitter.current_line = child_line;
                    method_emitter.line_starts = child_line_starts;
                    method_emitter.file_path = self.file_path.clone();
                    method_emitter.current_class = Some(class_sym);
                    method_emitter.current_namespace = self.current_namespace;
                    method_emitter.chunk.strict_types = self.chunk.strict_types;

                    // Build method name after creating method_emitter to avoid borrow issues
                    let method_name_full = {
                        let class_name = method_emitter.interner.lookup(class_sym).unwrap_or(b"");
                        let mut full = class_name.to_vec();
                        full.extend_from_slice(b"::");
                        full.extend_from_slice(method_name_str);
                        method_emitter.interner.intern(&full)
                    };
                    method_emitter.current_function = Some(method_name_full);

                    // 3. Process params
                    let mut param_syms = Vec::new();
                    for (i, info) in param_infos.iter().enumerate() {
                        let p_name = method_emitter.get_text(info.name_span);
                        if p_name == b"$this" {
                            method_emitter.compile_error(info.name_span.start, "Cannot use $this as parameter");
                        }
                        if p_name.starts_with(b"$") {
                            let sym = method_emitter.interner.intern(&p_name[1..]);
                            let param_type = info.ty.and_then(|ty| method_emitter.convert_type(ty));
                            let default_value = info.default.map(|expr| method_emitter.const_initializer(expr));

                            param_syms.push(FuncParam {
                                name: sym,
                                by_ref: info.by_ref,
                                param_type,
                                is_variadic: info.variadic,
                                default_value: default_value.clone(),
                                default_display: info.default.map(|e| method_emitter.default_display(e).into()),
                                attributes: method_emitter.build_attribute_instances(info.attributes),
                            });

                            if let Some(val) = default_value.clone() {
                                let val = val;
                                let idx = method_emitter.add_constant(val);
                                method_emitter
                                    .chunk
                                    .code
                                    .push(OpCode::RecvInit(i as u32, idx as u16));
                            } else {
                                method_emitter.chunk.code.push(if info.variadic {
                                    OpCode::RecvVariadic(i as u32)
                                } else {
                                    OpCode::Recv(i as u32)
                                });
                            }
                        }
                    }

                    // Constructor promotion: `__construct(public $x)` declares the
                    // property and assigns it before the body runs.
                    let is_constructor = method_name_str.eq_ignore_ascii_case(b"__construct");
                    let this_sym = method_emitter.interner.intern(b"this");
                    for param in params.iter().filter(|p| is_constructor && !p.modifiers.is_empty()) {
                        let prop_sym = method_emitter.interner.intern(&method_emitter.get_text(param.name.span)[1..]);
                        method_emitter.chunk.code.push(OpCode::LoadVar(this_sym));
                        method_emitter.chunk.code.push(OpCode::LoadVar(prop_sym));
                        method_emitter.chunk.code.push(OpCode::AssignProp(prop_sym));
                        method_emitter.chunk.code.push(OpCode::Pop);
                    }

                    let (mut method_chunk, is_generator) = method_emitter.compile(body);
                    method_chunk.returns_ref = *by_ref;

                    for param in params.iter().filter(|p| is_constructor && !p.modifiers.is_empty()) {
                        let prop_sym = self.interner.intern(&self.get_text(param.name.span)[1..]);
                        let is_readonly = param.modifiers.iter().any(|t| t.kind == TokenKind::Readonly);
                        let type_hint = param.ty.and_then(|t| self.convert_type(t));
                        // A typed (or readonly) promoted property is uninitialized until the constructor runs.
                        let default = if type_hint.is_some() || is_readonly {
                            Val::Uninitialized
                        } else {
                            Val::Null
                        };
                        let default_idx = self.add_constant(default);
                        let type_hint_idx = match type_hint {
                            Some(th) => self.add_constant(Val::Resource(Rc::new(th))),
                            None => self.add_constant(Val::Null),
                        };
                        self.chunk.code.push(OpCode::DefProp(
                            class_sym,
                            prop_sym,
                            default_idx as u16,
                            self.get_visibility(param.modifiers),
                            type_hint_idx as u32,
                            is_readonly,
                        ));
                    }

                    // Convert return type
                    let ret_type = return_type.and_then(|rt| self.convert_type(rt));

                    let user_func = UserFunc {
                        params: param_syms,
                        uses: Vec::new(),
                        chunk: Rc::new(method_chunk),
                        is_static,
                        is_generator,
                        statics: Rc::new(RefCell::new(HashMap::new())),
                        return_type: ret_type,
                        attributes: Vec::new(),
                        decl_line: self.line_at(name.span.start),
                        is_final: modifiers.iter().any(|t| t.kind == TokenKind::Final),
                    };

                    // Store in constants
                    let func_res = Val::Resource(Rc::new(user_func));
                    let const_idx = self.add_constant(func_res);

                    self.chunk.code.push(OpCode::DefMethod(
                        class_sym,
                        method_sym,
                        const_idx as u32,
                        visibility,
                        is_static,
                        is_abstract,
                    ));

                    for attr_idx in self.build_attribute_constants(attributes) {
                        self.chunk.code.push(OpCode::AddMethodAttribute(
                            class_sym,
                            method_sym,
                            attr_idx,
                        ));
                    }
                }
                ClassMember::Property {
                    entries,
                    modifiers,
                    ty,
                    attributes,
                    ..
                } => {
                    let visibility = self.get_visibility(modifiers);
                    let is_static = modifiers.iter().any(|t| t.kind == TokenKind::Static);
                    let is_readonly = modifiers.iter().any(|t| t.kind == TokenKind::Readonly);
                    let attr_indices = self.build_attribute_constants(attributes);

                    // Extract type hint once for all properties in this declaration
                    let type_hint_opt = ty.and_then(|t| self.convert_type(t));
                    let type_hint_idx = if let Some(ref th) = type_hint_opt {
                        self.add_constant(Val::Resource(Rc::new(th.clone())))
                    } else {
                        self.add_constant(Val::Null)
                    };

                    for entry in *entries {
                        let prop_name_str = self.get_text(entry.name.span);
                        let prop_name = if prop_name_str.starts_with(b"$") {
                            &prop_name_str[1..]
                        } else {
                            prop_name_str
                        };
                        let prop_sym = self.interner.intern(prop_name);

                        let default_idx = if let Some(default_expr) = entry.default {
                            let val = self.const_initializer(default_expr);
                            self.add_constant(val)
                        } else {
                            if type_hint_opt.is_some() || is_readonly {
                                self.add_constant(Val::Uninitialized)
                            } else {
                                self.add_constant(Val::Null)
                            }
                        };

                        if is_static {
                            self.chunk.code.push(OpCode::DefStaticProp(
                                class_sym,
                                prop_sym,
                                default_idx as u16,
                                visibility,
                                type_hint_idx as u32,
                            ));
                        } else {
                            self.chunk.code.push(OpCode::DefProp(
                                class_sym,
                                prop_sym,
                                default_idx as u16,
                                visibility,
                                type_hint_idx as u32,
                                is_readonly,
                            ));
                        }

                        for &attr_idx in &attr_indices {
                            self.chunk.code.push(OpCode::AddPropertyAttribute(
                                class_sym,
                                prop_sym,
                                attr_idx,
                                is_static,
                            ));
                        }
                    }
                }
                // Hooks do not run yet (#26); the property itself is declared
                // so it exists on instances. No backing value: null.
                ClassMember::PropertyHook {
                    modifiers, ty, name, default, ..
                } => {
                    let visibility = self.get_visibility(modifiers);
                    let is_readonly = modifiers.iter().any(|t| t.kind == TokenKind::Readonly);
                    let type_hint_idx = match ty.and_then(|t| self.convert_type(t)) {
                        Some(th) => self.add_constant(Val::Resource(Rc::new(th))),
                        None => self.add_constant(Val::Null),
                    };
                    let name_text = self.get_text(name.span);
                    let prop_sym = self.interner.intern(name_text.strip_prefix(b"$").unwrap_or(name_text));
                    let val = match default {
                        Some(d) => self.const_initializer(d),
                        None => Val::Null,
                    };
                    let default_idx = self.add_constant(val);
                    self.chunk.code.push(OpCode::DefProp(
                        class_sym,
                        prop_sym,
                        default_idx as u16,
                        visibility,
                        type_hint_idx as u32,
                        is_readonly,
                    ));
                }
                ClassMember::Const {
                    consts,
                    modifiers,
                    attributes,
                    ..
                } => {
                    let visibility = self.get_visibility(modifiers);
                    let attr_indices = self.build_attribute_constants(attributes);
                    for entry in *consts {
                        let const_name_str = self.get_text(entry.name.span);
                        let const_sym = self.interner.intern(const_name_str);

                        let val = self.const_initializer(entry.value);
                        let val_idx = self.add_constant(val);
                        self.chunk.code.push(OpCode::DefClassConst(
                            class_sym,
                            const_sym,
                            val_idx as u16,
                            visibility,
                        ));

                        for &attr_idx in &attr_indices {
                            self.chunk.code.push(OpCode::AddClassConstAttribute(
                                class_sym,
                                const_sym,
                                attr_idx,
                            ));
                        }
                    }
                }
                ClassMember::Case { name, value, .. } => {
                    let case_name_str = self.get_text(name.span);
                    let case_sym = self.interner.intern(case_name_str);
                    let (val_idx, has_value) = if let Some(expr) = value {
                        let val = self.eval_constant_expr(expr);
                        (self.add_constant(val) as u16, true)
                    } else {
                        (0u16, false)
                    };

                    self.chunk.code.push(OpCode::DefEnumCase(
                        class_sym,
                        case_sym,
                        val_idx,
                        has_value,
                    ));
                }
                ClassMember::TraitUse { traits, .. } => {
                    for trait_name in *traits {
                        let trait_sym = self.class_sym(trait_name.span);
                        self.chunk.code.push(OpCode::UseTrait(class_sym, trait_sym));
                    }
                }
            }
        }
    }

    /// Declare a class body; shared by `class Foo {}` and `new class {}`.
    fn emit_class_decl(
        &mut self,
        class_sym: Symbol,
        extends: &Option<crate::parser::ast::Name>,
        implements: &[crate::parser::ast::Name],
        attributes: &[AttributeGroup],
        modifiers: &[Token],
        members: &[ClassMember],
    ) {
        let class_line = self.current_line;
        let parent_sym = extends
            .as_ref()
            .map(|parent| self.class_sym(parent.span));
        self.chunk
            .code
            .push(OpCode::DefClass(class_sym, parent_sym));

        if modifiers.iter().any(|m| m.kind == TokenKind::Abstract) {
            self.chunk.code.push(OpCode::MarkAbstract(class_sym));
        }
        if modifiers.iter().any(|m| m.kind == TokenKind::Final) {
            self.chunk.code.push(OpCode::MarkFinal(class_sym));
        }

        for interface in implements {
            let interface_sym = self.class_sym(interface.span);
            self.chunk
                .code
                .push(OpCode::AddInterface(class_sym, interface_sym));
        }

        // Check for #[AllowDynamicProperties] attribute
        for attr_group in attributes {
            for attr in attr_group.attributes {
                let attr_name = self.get_text(attr.name.span);
                // Check for both fully qualified and simple name
                if attr_name == b"AllowDynamicProperties"
                    || attr_name.ends_with(b"\\AllowDynamicProperties")
                {
                    self.chunk
                        .code
                        .push(OpCode::AllowDynamicProperties(class_sym));
                    break;
                }
            }
        }

        self.emit_class_attributes(class_sym, attributes);

        // Track class context while emitting members
        let prev_class = self.current_class;
        self.current_class = Some(class_sym);
        self.emit_members(class_sym, members);
        self.current_class = prev_class;

        // Finalize class: link it (inheritance checks), on the class's line.
        self.sync_lines();
        let saved = self.current_line;
        self.current_line = class_line;
        self.chunk.code.push(OpCode::FinalizeClass(class_sym));
        self.sync_lines();
        self.current_line = saved;
    }

    fn emit_class_attributes(&mut self, class_sym: Symbol, attributes: &[AttributeGroup]) {
        for attr_group in attributes {
            for attr in attr_group.attributes {
                let attr_sym = self.class_sym(attr.name.span);
                let mut args = Vec::new();

                for arg in attr.args {
                    let val = self.eval_constant_expr(arg.value);
                    let name = arg
                        .name
                        .map(|token| self.interner.intern(self.get_text(token.span)));
                    args.push(AttributeArg { name, value: val });
                }

                let instance = AttributeInstance { name: attr_sym, args };
                let const_idx = self.add_constant(Val::Resource(Rc::new(instance)));
                self.chunk
                    .code
                    .push(OpCode::AddClassAttribute(class_sym, const_idx as u16));
            }
        }
    }

    fn build_attribute_constants(&mut self, attributes: &[AttributeGroup]) -> Vec<u16> {
        let mut indices = Vec::new();
        for attr_group in attributes {
            for attr in attr_group.attributes {
                let attr_sym = self.class_sym(attr.name.span);
                let mut args = Vec::new();

                for arg in attr.args {
                    let val = self.eval_constant_expr(arg.value);
                    let name = arg
                        .name
                        .map(|token| self.interner.intern(self.get_text(token.span)));
                    args.push(AttributeArg { name, value: val });
                }

                let instance = AttributeInstance { name: attr_sym, args };
                let const_idx = self.add_constant(Val::Resource(Rc::new(instance)));
                indices.push(const_idx as u16);
            }
        }
        indices
    }

    fn build_attribute_instances(
        &mut self,
        attributes: &[AttributeGroup],
    ) -> Vec<AttributeInstance> {
        let mut instances = Vec::new();
        for attr_group in attributes {
            for attr in attr_group.attributes {
                let attr_sym = self.class_sym(attr.name.span);
                let mut args = Vec::new();

                for arg in attr.args {
                    let val = self.eval_constant_expr(arg.value);
                    let name = arg
                        .name
                        .map(|token| self.interner.intern(self.get_text(token.span)));
                    args.push(AttributeArg { name, value: val });
                }

                instances.push(AttributeInstance {
                    name: attr_sym,
                    args,
                });
            }
        }
        instances
    }

    fn emit_stmt(&mut self, stmt: &Stmt) {
        self.at_line(stmt.span().start, |this| this.emit_stmt_inner(stmt));
    }

    fn emit_stmt_inner(&mut self, stmt: &Stmt) {
        if let Some((offset, message)) = self.stmt_this_misuse(stmt) {
            self.compile_error(offset, message);
            return;
        }
        if matches!(stmt, Stmt::Function { .. } | Stmt::Class { .. })
            && self.hoisted.contains(&stmt.span().start)
        {
            return;
        }
        let declared = match stmt {
            Stmt::Class { name, .. } => Some((name, "a class name")),
            Stmt::Interface { name, .. } => Some((name, "an interface name")),
            Stmt::Trait { name, .. } => Some((name, "a trait name")),
            Stmt::Enum { name, .. } => Some((name, "an enum name")),
            _ => None,
        };
        if let Some((name, kind)) = declared {
            // zend_assert_valid_class_name
            let text = self.get_text(name.span);
            if Self::is_reserved_class_name(text) {
                let message = format!(
                    "Cannot use \"{}\" as {} as it is reserved",
                    String::from_utf8_lossy(text),
                    kind
                );
                self.compile_error(name.span.start, message);
                return;
            }
            if text == b"_" {
                let message = format!("Using \"_\" as {} is deprecated since 8.4", kind);
                self.compile_warning(ErrorLevel::Deprecated, name.span.start, message);
            }
        }
        match stmt {
            Stmt::Namespace { name, body, .. } => {
                self.enter_namespace(name.as_ref());
                if let Some(body) = body {
                    for stmt in *body {
                        self.emit_stmt(stmt);
                    }
                    self.enter_namespace(None);
                }
            }
            Stmt::Use { uses, kind, .. } => self.add_imports(uses, *kind, true),
            Stmt::Declare { declares, body, .. } => {
                // PHP: declare(strict_types=1) is per-file and affects calls made from this file.
                // The parser already validates strict_types is an integer literal 0/1.
                for item in *declares {
                    let key = self.get_text(item.key.span);
                    if key.eq_ignore_ascii_case(b"strict_types")
                        && let Expr::Integer { value, .. } = item.value
                    {
                        self.chunk.strict_types = integer_literal(value) == Val::Int(1);
                    }
                }

                for s in *body {
                    self.emit_stmt(s);
                }
            }
            Stmt::Echo { exprs, .. } => {
                for expr in *exprs {
                    self.emit_expr(expr);
                    self.chunk.code.push(OpCode::Echo);
                }
            }
            Stmt::InlineHtml { value, .. } => {
                // Output inline HTML/text that appears outside PHP tags
                // Convert the raw bytes to a string constant and echo it
                let idx = self.add_constant(Val::String(value.to_vec().into()));
                self.chunk.code.push(OpCode::Const(idx as u16));
                self.chunk.code.push(OpCode::Echo);
            }
            Stmt::Expression { expr, .. } => {
                self.emit_expr(expr);
                self.chunk.code.push(OpCode::Pop);
            }
            Stmt::Return { expr, .. } => {
                if let Some(e) = expr {
                    self.emit_expr(e);
                } else {
                    let idx = self.add_constant(Val::Null);
                    self.chunk.code.push(OpCode::Const(idx as u16));
                }
                // Return type checking is now done in the Return handler
                self.chunk.code.push(OpCode::Return);
            }
            Stmt::Const { consts, .. } => {
                for c in *consts {
                    let name_full = self.prefix_namespace(self.get_text(c.name.span));
                    let sym = self.interner.intern(&name_full);

                    // Value must be constant expression.
                    // For now, we only support literals or simple expressions we can evaluate at compile time?
                    // Or we can emit code to evaluate it and then define it?
                    // PHP `const` requires constant expression.
                    // If we emit code, we can use `DefGlobalConst` which takes a value index?
                    // No, `DefGlobalConst` takes `val_idx` which implies it's in the constant table.
                    // So we must evaluate it at compile time.

                    let val = self.const_initializer(c.value);

                    let val_idx = self.add_constant(val);
                    self.chunk
                        .code
                        .push(OpCode::DefGlobalConst(sym, val_idx as u16));
                }
            }
            Stmt::Global { vars, .. } => {
                for var in *vars {
                    if let Expr::Variable { span, .. } = var {
                        let name = self.get_text(*span);
                        if name.starts_with(b"$") {
                            let var_name = &name[1..];
                            let sym = self.interner.intern(var_name);
                            self.chunk.code.push(OpCode::BindGlobal(sym));
                        }
                    }
                }
            }
            Stmt::Static { vars, .. } => {
                for var in *vars {
                    // Check if var.var is Assign
                    let (target_var, default_expr) = if let Expr::Assign {
                        var: assign_var,
                        expr: assign_expr,
                        ..
                    } = var.var
                    {
                        (*assign_var, Some(*assign_expr))
                    } else {
                        (var.var, var.default)
                    };

                    let name = if let Expr::Variable { span, .. } = target_var {
                        let name = self.get_text(*span);
                        if name.starts_with(b"$") {
                            self.interner.intern(&name[1..])
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    };

                    match default_expr {
                        // The initializer runs once, the first time the
                        // statement runs (any expression since PHP 8.3).
                        Some(expr) => {
                            let check = self.chunk.code.len();
                            self.chunk.code.push(OpCode::BindStaticCheck(name, 0));
                            self.emit_expr(expr);
                            self.chunk.code.push(OpCode::BindStaticSet(name));
                            let end = self.chunk.code.len() as u32;
                            self.chunk.code[check] = OpCode::BindStaticCheck(name, end);
                        }
                        None => {
                            let idx = self.add_constant(Val::Null);
                            self.chunk.code.push(OpCode::BindStatic(name, idx as u16));
                        }
                    }
                }
            }
            Stmt::Unset { vars, .. } => {
                for var in *vars {
                    match var {
                        Expr::Variable { span, .. } => {
                            let name = self.get_text(*span);
                            if name.starts_with(b"$") {
                                let sym = self.interner.intern(&name[1..]);
                                self.chunk.code.push(OpCode::UnsetVar(sym));
                            }
                        }
                        Expr::IndirectVariable { name, .. } => {
                            self.emit_expr(name);
                            self.chunk.code.push(OpCode::UnsetVarDynamic);
                        }
                        Expr::ArrayDimFetch { array, dim, .. } => {
                            if let Expr::Variable { span, .. } = array {
                                let name = self.get_text(*span);
                                if name.starts_with(b"$") {
                                    let sym = self.interner.intern(&name[1..]);
                                    self.chunk.code.push(OpCode::LoadVar(sym));
                                    self.chunk.code.push(OpCode::Dup);

                                    if let Some(d) = dim {
                                        self.emit_expr(d);
                                    } else {
                                        let idx = self.add_constant(Val::Null);
                                        self.chunk.code.push(OpCode::Const(idx as u16));
                                    }

                                    self.chunk.code.push(OpCode::UnsetDim);
                                    self.chunk.code.push(OpCode::StoreVar(sym));
                                }
                            } else {
                                // Check if this is a property fetch (possibly nested): $obj->prop['key']['key2']...
                                // Use flatten_dim_fetch to get all keys
                                let (base, keys) = Self::flatten_dim_fetch(var);

                                // Check if the base is a PropertyFetch
                                if let Expr::PropertyFetch {
                                    target, property, ..
                                } = base
                                {
                                    // Ensure we have at least one key
                                    if keys.is_empty() {
                                        return; // Shouldn't happen for ArrayDimFetch
                                    }

                                    // Get property name symbol
                                    let prop_sym = if let Expr::Variable { span, .. } = property {
                                        let name = self.get_text(*span);
                                        self.interner.intern(name)
                                    } else {
                                        return; // Can't handle dynamic property names in unset yet
                                    };

                                    // Emit target (obj)
                                    self.emit_write_base(target); // [obj]
                                    self.chunk.code.push(OpCode::Dup); // [obj, obj]

                                    // Fetch the property
                                    self.chunk.code.push(OpCode::FetchProp(prop_sym)); // [obj, array]

                                    // Emit all keys
                                    for key in &keys {
                                        if let Some(k) = key {
                                            self.emit_expr(k);
                                        } else {
                                            let idx = self.add_constant(Val::Null);
                                            self.chunk.code.push(OpCode::Const(idx as u16));
                                        }
                                    }

                                    // Unset nested dimension
                                    self.chunk
                                        .code
                                        .push(OpCode::UnsetNestedDim(keys.len() as u8)); // [obj, modified_array]

                                    // Assign back to property
                                    self.chunk.code.push(OpCode::AssignProp(prop_sym)); // []
                                    self.chunk.code.push(OpCode::Pop); // discard result
                                }
                            }
                        }
                        Expr::PropertyFetch {
                            target, property, ..
                        } => {
                            self.emit_write_base(target);
                            if let Expr::Variable { span, .. } = property {
                                let name = self.get_text(*span);
                                let idx = self.add_constant(Val::String(name.to_vec().into()));
                                self.chunk.code.push(OpCode::Const(idx as u16));
                                self.chunk.code.push(OpCode::UnsetObj);
                            }
                        }
                        Expr::ClassConstFetch {
                            class, constant, ..
                        } => {
                            let is_static_prop = if let Expr::Variable { span, .. } = constant {
                                let name = self.get_text(*span);
                                name.starts_with(b"$")
                            } else {
                                false
                            };

                            if is_static_prop
                                && let Expr::Variable { span, .. } = class
                            {
                                let name = &self.resolve_class_name(self.raw_text(*span));
                                if !name.starts_with(b"$") {
                                    let idx =
                                        self.add_constant(Val::String(name.to_vec().into()));
                                    self.chunk.code.push(OpCode::Const(idx as u16));
                                } else {
                                    let sym = self.interner.intern(&name[1..]);
                                    self.chunk.code.push(OpCode::LoadVar(sym));
                                }

                                if let Expr::Variable {
                                    span: prop_span, ..
                                } = constant
                                {
                                    let prop_name = self.get_text(*prop_span);
                                    let idx = self.add_constant(Val::String(
                                        prop_name[1..].to_vec().into(),
                                    ));
                                    self.chunk.code.push(OpCode::Const(idx as u16));
                                    self.chunk.code.push(OpCode::UnsetStaticProp);
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Stmt::Break { level, .. } => {
                // Determine the break level (default 1)
                let break_level = if let Some(level_expr) = level {
                    // Try to evaluate as a constant integer
                    self.get_literal_value(level_expr)
                        .and_then(|v| match v {
                            Val::Int(i) if i > 0 => Some(i as usize),
                            _ => None,
                        })
                        .unwrap_or(1)
                } else {
                    1
                };

                // Find the target loop (counting from innermost)
                let loop_depth = self.loop_stack.len();
                if break_level > 0 && break_level <= loop_depth {
                    // Calculate which loop to target (from the end of the stack)
                    let target_loop_idx = loop_depth - break_level;

                    self.emit_loop_exit_pops(target_loop_idx, true);
                    let idx = self.chunk.code.len();
                    // Check if we're inside try-finally blocks
                    if !self.try_finally_stack.is_empty() {
                        // Use JmpFinally which will execute finally blocks at runtime
                        self.chunk.code.push(OpCode::JmpFinally(0)); // Patch later
                    } else {
                        // Normal jump
                        self.chunk.code.push(OpCode::Jmp(0)); // Patch later
                    }

                    // Register the jump with the target loop
                    self.loop_stack[target_loop_idx].break_jumps.push(idx);
                }
            }
            Stmt::Continue { level, .. } => {
                // Determine the continue level (default 1)
                let continue_level = if let Some(level_expr) = level {
                    // Try to evaluate as a constant integer
                    self.get_literal_value(level_expr)
                        .and_then(|v| match v {
                            Val::Int(i) if i > 0 => Some(i as usize),
                            _ => None,
                        })
                        .unwrap_or(1)
                } else {
                    1
                };

                // Find the target loop (counting from innermost)
                let loop_depth = self.loop_stack.len();
                if continue_level > 0 && continue_level <= loop_depth {
                    // Calculate which loop to target (from the end of the stack)
                    let target_loop_idx = loop_depth - continue_level;

                    self.emit_loop_exit_pops(target_loop_idx, false);
                    let idx = self.chunk.code.len();
                    // Check if we're inside try-finally blocks
                    if !self.try_finally_stack.is_empty() {
                        // Use JmpFinally which will execute finally blocks at runtime
                        self.chunk.code.push(OpCode::JmpFinally(0)); // Patch later
                    } else {
                        // Normal jump
                        self.chunk.code.push(OpCode::Jmp(0)); // Patch later
                    }

                    // Register the jump with the target loop
                    self.loop_stack[target_loop_idx].continue_jumps.push(idx);
                }
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                self.emit_expr(condition);

                let jump_false_idx = self.chunk.code.len();
                self.chunk.code.push(OpCode::JmpIfFalse(0));

                for stmt in *then_block {
                    self.emit_stmt(stmt);
                }

                let jump_end_idx = self.chunk.code.len();
                self.chunk.code.push(OpCode::Jmp(0));

                let else_label = self.chunk.code.len();
                self.patch_jump(jump_false_idx, else_label);

                if let Some(else_stmts) = else_block {
                    for stmt in *else_stmts {
                        self.emit_stmt(stmt);
                    }
                }

                let end_label = self.chunk.code.len();
                self.patch_jump(jump_end_idx, end_label);
            }
            Stmt::Function {
                attributes,
                name,
                params,
                body,
                by_ref,
                return_type,
                ..
            } => {
                let func_name = self.prefix_namespace(self.get_text(name.span));
                let func_sym = self.interner.intern(&func_name);

                // 1. Collect param info to avoid borrow issues
                    struct ParamInfo<'a> {
                        name_span: crate::parser::span::Span,
                        by_ref: bool,
                        ty: Option<&'a Type<'a>>,
                        default: Option<&'a Expr<'a>>,
                        variadic: bool,
                        attributes: &'a [AttributeGroup<'a>],
                    }

                let mut param_infos = Vec::new();
                for param in *params {
                    param_infos.push(ParamInfo {
                        name_span: param.name.span,
                        by_ref: param.by_ref,
                        ty: param.ty,
                        default: param.default.as_ref().map(|e| *e),
                        variadic: param.variadic,
                        attributes: param.attributes,
                    });
                }

                // 2. Create emitter with inherited context
                let (child_line, child_line_starts) = self.child_lines(name.span.start);
                    let compile_error = self.compile_error.clone();
                    let mut func_emitter = Emitter::new(self.source, self.interner);
                    func_emitter.compile_error = compile_error;
                    func_emitter.compile_warnings = self.compile_warnings.clone();
                    func_emitter.imports = self.imports.clone();
                    func_emitter.current_line = child_line;
                    func_emitter.line_starts = child_line_starts;
                func_emitter.file_path = self.file_path.clone();
                func_emitter.current_function = Some(func_sym);
                func_emitter.current_namespace = self.current_namespace;
                func_emitter.chunk.strict_types = self.chunk.strict_types;

                // 3. Process params using func_emitter
                let mut param_syms = Vec::new();
                for (i, info) in param_infos.iter().enumerate() {
                    let p_name = func_emitter.get_text(info.name_span);
                    if p_name == b"$this" {
                        func_emitter.compile_error(info.name_span.start, "Cannot use $this as parameter");
                    }
                    if p_name.starts_with(b"$") {
                        let sym = func_emitter.interner.intern(&p_name[1..]);
                        let param_type = info.ty.and_then(|ty| func_emitter.convert_type(ty));
                        let default_value = info.default.map(|expr| func_emitter.const_initializer(expr));

                        param_syms.push(FuncParam {
                            name: sym,
                            by_ref: info.by_ref,
                            param_type,
                            is_variadic: info.variadic,
                            default_value: default_value.clone(),
                            default_display: info.default.map(|e| func_emitter.default_display(e).into()),
                            attributes: func_emitter.build_attribute_instances(info.attributes),
                        });

                        if let Some(val) = default_value.clone() {
                            let val = val;
                            let idx = func_emitter.add_constant(val);
                            func_emitter
                                .chunk
                                .code
                                .push(OpCode::RecvInit(i as u32, idx as u16));
                        } else {
                            func_emitter.chunk.code.push(if info.variadic {
                                OpCode::RecvVariadic(i as u32)
                            } else {
                                OpCode::Recv(i as u32)
                            });
                        }
                    }
                }

                let (mut func_chunk, is_generator) = func_emitter.compile(body);
                func_chunk.returns_ref = *by_ref;

                // Convert return type
                let ret_type = return_type.and_then(|rt| self.convert_type(rt));

                let user_func = UserFunc {
                    params: param_syms,
                    uses: Vec::new(),
                    chunk: Rc::new(func_chunk),
                    is_static: false,
                    is_generator,
                    statics: Rc::new(RefCell::new(HashMap::new())),
                    return_type: ret_type,
                    attributes: self.build_attribute_instances(attributes),
                    decl_line: self.line_at(name.span.start),
                    is_final: false,
                };

                let func_res = Val::Resource(Rc::new(user_func));
                let const_idx = self.add_constant(func_res);

                self.chunk
                    .code
                    .push(OpCode::DefFunc(func_sym, const_idx as u32));
            }
            Stmt::Class {
                name,
                members,
                extends,
                implements,
                attributes,
                modifiers,
                ..
            } => {
                let class_name = self.prefix_namespace(self.get_text(name.span));
                let class_sym = self.interner.intern(&class_name);
                self.emit_class_decl(class_sym, extends, implements, attributes, modifiers, members);
            }
            Stmt::Interface {
                name,
                members,
                extends,
                ..
            } => {
                let name_full = self.prefix_namespace(self.get_text(name.span));
                let sym = self.interner.intern(&name_full);
                let interface_line = self.current_line;

                self.chunk.code.push(OpCode::DefInterface(sym));

                for interface in *extends {
                    let interface_sym = self.class_sym(interface.span);
                    self.chunk
                        .code
                        .push(OpCode::AddInterface(sym, interface_sym));
                }

                let prev_class = self.current_class;
                self.current_class = Some(sym);
                self.emit_members(sym, members);
                self.current_class = prev_class;

                self.sync_lines();
                let saved = self.current_line;
                self.current_line = interface_line;
                self.chunk.code.push(OpCode::FinalizeClass(sym));
                self.sync_lines();
                self.current_line = saved;
            }
            Stmt::Trait { name, members, .. } => {
                let name_full = self.prefix_namespace(self.get_text(name.span));
                let sym = self.interner.intern(&name_full);

                self.chunk.code.push(OpCode::DefTrait(sym));

                let prev_trait = self.current_trait;
                self.current_trait = Some(sym);
                self.emit_members(sym, members);
                self.current_trait = prev_trait;
            }
            Stmt::Enum {
                name,
                backed_type,
                implements,
                members,
                attributes,
                ..
            } => {
                let enum_name = self.prefix_namespace(self.get_text(name.span));
                let enum_sym = self.interner.intern(&enum_name);

                let backed_flag = match backed_type {
                    Some(Type::Simple(tok)) if tok.kind == TokenKind::TypeInt => 1,
                    Some(Type::Simple(tok)) if tok.kind == TokenKind::TypeString => 2,
                    _ => 0,
                };

                self.chunk.code.push(OpCode::DefEnum(enum_sym, backed_flag));

                for interface in *implements {
                    let interface_sym = self.class_sym(interface.span);
                    self.chunk
                        .code
                        .push(OpCode::AddInterface(enum_sym, interface_sym));
                }

                self.emit_class_attributes(enum_sym, attributes);

                let prev_class = self.current_class;
                self.current_class = Some(enum_sym);
                self.emit_members(enum_sym, members);
                self.current_class = prev_class;

                self.chunk.code.push(OpCode::FinalizeClass(enum_sym));
            }

            Stmt::While {
                condition, body, ..
            } => {
                let start_label = self.chunk.code.len();

                self.emit_expr(condition);

                let end_jump = self.chunk.code.len();
                self.chunk.code.push(OpCode::JmpIfFalse(0)); // Patch later

                self.loop_stack.push(LoopInfo {
                    break_jumps: Vec::new(),
                    continue_jumps: Vec::new(),
                    stack_slots: 0,
                    end_pops: false,
                });

                for stmt in *body {
                    self.emit_stmt(stmt);
                }

                self.chunk.code.push(OpCode::Jmp(start_label as u32));

                let end_label = self.chunk.code.len();
                self.chunk.code[end_jump] = OpCode::JmpIfFalse(end_label as u32);

                let loop_info = self.loop_stack.pop().unwrap();
                for idx in loop_info.break_jumps {
                    self.patch_jump(idx, end_label);
                }
                for idx in loop_info.continue_jumps {
                    self.patch_jump(idx, start_label);
                }
            }
            Stmt::DoWhile {
                body, condition, ..
            } => {
                let start_label = self.chunk.code.len();

                self.loop_stack.push(LoopInfo {
                    break_jumps: Vec::new(),
                    continue_jumps: Vec::new(),
                    stack_slots: 0,
                    end_pops: false,
                });

                for stmt in *body {
                    self.emit_stmt(stmt);
                }

                let continue_label = self.chunk.code.len();
                self.emit_expr(condition);
                self.chunk.code.push(OpCode::JmpIfTrue(start_label as u32));

                let end_label = self.chunk.code.len();

                let loop_info = self.loop_stack.pop().unwrap();
                for idx in loop_info.break_jumps {
                    self.patch_jump(idx, end_label);
                }
                for idx in loop_info.continue_jumps {
                    self.patch_jump(idx, continue_label);
                }
            }
            Stmt::For {
                init,
                condition,
                loop_expr,
                body,
                ..
            } => {
                for expr in *init {
                    self.emit_expr(expr);
                    self.chunk.code.push(OpCode::Pop); // Discard result
                }

                let start_label = self.chunk.code.len();

                let mut end_jump = None;
                if !condition.is_empty() {
                    for (i, expr) in condition.iter().enumerate() {
                        self.emit_expr(expr);
                        if i < condition.len() - 1 {
                            self.chunk.code.push(OpCode::Pop);
                        }
                    }
                    end_jump = Some(self.chunk.code.len());
                    self.chunk.code.push(OpCode::JmpIfFalse(0)); // Patch later
                }

                self.loop_stack.push(LoopInfo {
                    break_jumps: Vec::new(),
                    continue_jumps: Vec::new(),
                    stack_slots: 0,
                    end_pops: false,
                });

                for stmt in *body {
                    self.emit_stmt(stmt);
                }

                let continue_label = self.chunk.code.len();
                for expr in *loop_expr {
                    self.emit_expr(expr);
                    self.chunk.code.push(OpCode::Pop);
                }

                self.chunk.code.push(OpCode::Jmp(start_label as u32));

                let end_label = self.chunk.code.len();
                if let Some(idx) = end_jump {
                    self.chunk.code[idx] = OpCode::JmpIfFalse(end_label as u32);
                }

                let loop_info = self.loop_stack.pop().unwrap();
                for idx in loop_info.break_jumps {
                    self.patch_jump(idx, end_label);
                }
                for idx in loop_info.continue_jumps {
                    self.patch_jump(idx, continue_label);
                }
            }
            Stmt::Foreach {
                expr,
                key_var,
                value_var,
                body,
                ..
            } => {
                // Check if by-ref
                let is_by_ref = matches!(
                    value_var,
                    Expr::Unary {
                        op: UnaryOp::Reference,
                        ..
                    }
                );

                if is_by_ref {
                    if let Expr::Variable { span, .. } = expr {
                        let name = self.get_text(*span);
                        if name.starts_with(b"$") {
                            let sym = self.interner.intern(&name[1..]);
                            self.chunk.code.push(OpCode::MakeVarRef(sym));
                        } else {
                            self.emit_expr(expr);
                        }
                    } else {
                        self.emit_expr(expr);
                    }
                } else {
                    self.emit_expr(expr);
                }

                // IterInit(End)
                let init_idx = self.chunk.code.len();
                self.chunk.code.push(OpCode::IterInit(0)); // Patch later

                let start_label = self.chunk.code.len();

                // IterValid(End)
                let valid_idx = self.chunk.code.len();
                self.chunk.code.push(OpCode::IterValid(0)); // Patch later

                // IterGetVal
                if let Expr::Variable { span, .. } = value_var {
                    let name = self.get_text(*span);
                    if name.starts_with(b"$") {
                        let sym = self.interner.intern(&name[1..]);
                        self.chunk.code.push(OpCode::IterGetVal(sym));
                    }
                } else if let Expr::Unary {
                    op: UnaryOp::Reference,
                    expr,
                    ..
                } = value_var
                {
                    if let Expr::Variable { span, .. } = expr {
                        let name = self.get_text(*span);
                        if name.starts_with(b"$") {
                            let sym = self.interner.intern(&name[1..]);
                            self.chunk.code.push(OpCode::IterGetValRef(sym));
                        }
                    }
                }

                // IterGetKey
                if let Some(k) = key_var {
                    if let Expr::Variable { span, .. } = k {
                        let name = self.get_text(*span);
                        if name.starts_with(b"$") {
                            let sym = self.interner.intern(&name[1..]);
                            self.chunk.code.push(OpCode::IterGetKey(sym));
                        }
                    }
                }

                self.loop_stack.push(LoopInfo {
                    break_jumps: Vec::new(),
                    continue_jumps: Vec::new(),
                    stack_slots: 2, // iterable + position
                    end_pops: false,
                });

                // Body
                for stmt in *body {
                    self.emit_stmt(stmt);
                }

                let continue_label = self.chunk.code.len();
                // IterNext
                self.chunk.code.push(OpCode::IterNext);

                // Jump back to start
                self.chunk.code.push(OpCode::Jmp(start_label as u32));

                let end_label = self.chunk.code.len();

                // Patch jumps
                self.patch_jump(init_idx, end_label);
                self.patch_jump(valid_idx, end_label);

                let loop_info = self.loop_stack.pop().unwrap();
                for idx in loop_info.break_jumps {
                    self.patch_jump(idx, end_label);
                }
                for idx in loop_info.continue_jumps {
                    self.patch_jump(idx, continue_label);
                }
            }
            Stmt::Throw { expr, .. } => {
                self.emit_expr(expr);
                self.chunk.code.push(OpCode::Throw);
            }
            Stmt::Switch {
                condition, cases, ..
            } => {
                self.emit_expr(condition);

                let dispatch_jump = self.chunk.code.len();
                self.chunk.code.push(OpCode::Jmp(0)); // Patch later

                let mut case_labels = Vec::new();
                let mut default_label = None;

                self.loop_stack.push(LoopInfo {
                    break_jumps: Vec::new(),
                    continue_jumps: Vec::new(),
                    stack_slots: 1,
                    end_pops: true,
                });

                for case in *cases {
                    let label = self.chunk.code.len();
                    case_labels.push(label);

                    if case.condition.is_none() {
                        default_label = Some(label);
                    }

                    for stmt in case.body {
                        self.emit_stmt(stmt);
                    }
                }

                let jump_over_dispatch = self.chunk.code.len();
                self.chunk.code.push(OpCode::Jmp(0)); // Patch to end_label

                let dispatch_start = self.chunk.code.len();
                self.patch_jump(dispatch_jump, dispatch_start);

                // Dispatch logic
                for (i, case) in cases.iter().enumerate() {
                    if let Some(cond) = case.condition {
                        self.chunk.code.push(OpCode::Dup); // Dup switch cond
                        self.emit_expr(cond);
                        self.chunk.code.push(OpCode::IsEqual); // Loose comparison
                        self.chunk
                            .code
                            .push(OpCode::JmpIfTrue(case_labels[i] as u32));
                    }
                }

                // The subject stays on the stack while a case body runs (it
                // is the switch's one stack slot); it is popped at the end.
                if let Some(def_lbl) = default_label {
                    self.chunk.code.push(OpCode::Jmp(def_lbl as u32));
                } else {
                    // No default, jump to end
                    self.chunk.code.push(OpCode::Jmp(jump_over_dispatch as u32));
                    // Will be patched to end_label
                }

                let end_label = self.chunk.code.len();
                self.patch_jump(jump_over_dispatch, end_label);
                self.chunk.code.push(OpCode::Pop);

                let loop_info = self.loop_stack.pop().unwrap();
                for idx in loop_info.break_jumps {
                    self.patch_jump(idx, end_label);
                }
                // Continue in switch acts like break
                for idx in loop_info.continue_jumps {
                    self.patch_jump(idx, end_label);
                }
            }
            Stmt::Try {
                body,
                catches,
                finally,
                ..
            } => {
                let try_start = self.chunk.code.len() as u32;
                let stack_depth: u32 = self.loop_stack.iter().map(|l| l.stack_slots).sum();

                // If there's a finally block, we need to track it BEFORE emitting the try body
                // so that break/continue inside the try body know they're inside a final context
                let has_finally = finally.is_some();
                let try_finally_placeholder_idx = if has_finally {
                    // Reserve space in try_finally_stack with placeholder values
                    // We'll update them after we emit the finally block
                    self.try_finally_stack.push(TryFinallyInfo {
                        catch_table_idx: 0, // Will be updated
                        finally_start: 0,   // Will be updated
                        finally_end: 0,     // Will be updated
                    });
                    Some(self.try_finally_stack.len() - 1)
                } else {
                    None
                };

                for stmt in *body {
                    self.emit_stmt(stmt);
                }
                let try_end = self.chunk.code.len() as u32;

                // Jump from successful try to finally (or end if no finally)
                let jump_from_try = self.chunk.code.len();
                self.chunk.code.push(OpCode::Jmp(0)); // Will patch to finally or end

                let mut catch_jumps = Vec::new();
                let mut catch_ranges = Vec::new(); // Track catch block ranges for finally encoding

                for catch in *catches {
                    let catch_start = self.chunk.code.len() as u32;

                    for ty in catch.types {
                        let type_sym = self.class_sym(ty.span);

                        self.chunk.catch_table.push(CatchEntry {
                            stack_depth,
                            start: try_start,
                            end: try_end,
                            target: catch_start,
                            catch_type: Some(type_sym),
                            finally_target: None, // Will be set below if finally exists
                            finally_end: None,
                        });
                    }

                    if let Some(var) = catch.var {
                        let name = self.get_text(var.span);
                        if name.starts_with(b"$") {
                            let sym = self.interner.intern(&name[1..]);
                            self.chunk.code.push(OpCode::StoreVar(sym));
                        }
                    } else {
                        self.chunk.code.push(OpCode::Pop);
                    }

                    for stmt in catch.body {
                        self.emit_stmt(stmt);
                    }

                    let catch_end = self.chunk.code.len() as u32;
                    catch_ranges.push((catch_start, catch_end));

                    // Jump from catch to finally (or end if no finally)
                    catch_jumps.push(self.chunk.code.len());
                    self.chunk.code.push(OpCode::Jmp(0)); // Will patch to finally or end
                }

                // Emit finally block if present
                if let Some(finally_body) = finally {
                    let finally_start = self.chunk.code.len() as u32;
                    let catch_table_idx = self.chunk.catch_table.len();

                    // Update the placeholder in try_finally_stack
                    if let Some(idx) = try_finally_placeholder_idx {
                        self.try_finally_stack[idx].catch_table_idx = catch_table_idx;
                        self.try_finally_stack[idx].finally_start = finally_start;
                        // finally_end will be set after emitting
                    }

                    // Patch jump from try to finally
                    self.patch_jump(jump_from_try, finally_start as usize);

                    // Patch all catch block jumps to finally
                    for idx in &catch_jumps {
                        self.patch_jump(*idx, finally_start as usize);
                    }

                    // Emit the finally block statements
                    for stmt in *finally_body {
                        self.emit_stmt(stmt);
                    }
                    let finally_end = self.chunk.code.len() as u32;

                    // Update the finally_end in try_finally_stack
                    if let Some(idx) = try_finally_placeholder_idx {
                        self.try_finally_stack[idx].finally_end = finally_end;
                    }

                    // Update all existing catch entries to include finally_target and finally_end
                    // This enables unwinding through finally when exception is caught
                    for entry in self.chunk.catch_table.iter_mut() {
                        if entry.start == try_start && entry.end == try_end {
                            entry.finally_target = Some(finally_start);
                            entry.finally_end = Some(finally_end);
                        }
                    }

                    // Add a finally-only entry for the try block
                    // This ensures finally executes even on uncaught exceptions
                    self.chunk.catch_table.push(CatchEntry {
                        stack_depth,
                        start: try_start,
                        end: try_end,
                        target: finally_start,
                        catch_type: None, // No specific catch type - this is for finally
                        finally_target: None,
                        finally_end: Some(finally_end),
                    });

                    // Also add entries for catch blocks to ensure finally runs during their unwinding
                    for (catch_start, catch_end) in catch_ranges {
                        self.chunk.catch_table.push(CatchEntry {
                            stack_depth,
                            start: catch_start,
                            end: catch_end,
                            target: finally_start,
                            catch_type: None, // No specific catch type - this is for finally
                            finally_target: None,
                            finally_end: Some(finally_end),
                        });
                    }

                    // Pop from try_finally_stack after emitting
                    self.try_finally_stack.pop();

                    // Finally falls through to end
                } else {
                    // No finally - patch jumps directly to end
                    let after_catches = self.chunk.code.len();
                    self.patch_jump(jump_from_try, after_catches);
                    for idx in &catch_jumps {
                        self.patch_jump(*idx, after_catches);
                    }
                }
            }
            _ => {}
        }
    }

    fn patch_jump(&mut self, idx: usize, target: usize) {
        let op = self.chunk.code[idx];
        let new_op = match op {
            OpCode::Jmp(_) => OpCode::Jmp(target as u32),
            OpCode::JmpIfFalse(_) => OpCode::JmpIfFalse(target as u32),
            OpCode::JmpIfTrue(_) => OpCode::JmpIfTrue(target as u32),
            OpCode::JmpZEx(_) => OpCode::JmpZEx(target as u32),
            OpCode::JmpNzEx(_) => OpCode::JmpNzEx(target as u32),
            OpCode::Coalesce(_) => OpCode::Coalesce(target as u32),
            OpCode::JmpNull(_) => OpCode::JmpNull(target as u32),
            OpCode::IterInit(_) => OpCode::IterInit(target as u32),
            OpCode::IterValid(_) => OpCode::IterValid(target as u32),
            OpCode::JmpFinally(_) => OpCode::JmpFinally(target as u32),
            OpCode::DeclareAnonClass(sym, _) => OpCode::DeclareAnonClass(sym, target as u32),
            _ => panic!("Cannot patch non-jump opcode: {:?}", op),
        };
        self.chunk.code[idx] = new_op;
    }

    fn get_literal_value(&self, expr: &Expr) -> Option<Val> {
        match expr {
            Expr::Integer { value, .. } => Some(integer_literal(value)),
            Expr::String { value, .. } => {
                let s = string_literal(value);
                Some(Val::String(s.into()))
            }
            Expr::Boolean { value, .. } => Some(Val::Bool(*value)),
            Expr::Null { .. } => Some(Val::Null),
            Expr::Array { items, .. } => {
                if items.is_empty() {
                    Some(Val::Array(Rc::new(crate::core::value::ArrayData::new())))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn emit_expr(&mut self, expr: &Expr) {
        self.short_circuit(expr, |this| {
            this.at_line(expr.span().start, |this| this.emit_expr_inner(expr))
        });
    }

    /// The object of a property write (`$base->p = ...`, `$base->p++`):
    /// fetched for writing, so `$a[]->p = 1` is allowed.
    fn emit_write_base(&mut self, base: &Expr) {
        self.append_fetch_ok = true;
        self.emit_expr(base);
        self.append_fetch_ok = false;
    }

    /// The base of a chain link (`$a` in `$a->b`, `$a[1]`, `$a::$b`): part
    /// of the same short-circuit chain, so a `?->` inside it skips the
    /// rest of the chain.
    fn emit_chain_base(&mut self, base: &Expr) {
        self.at_line(base.span().start, |this| this.emit_expr_inner(base));
    }

    /// Emit `expr` with `emit`; if it heads a chain that contains `?->`,
    /// each `?->` that finds null jumps to the end of the chain, null
    /// being the chain's value (zend_short_circuiting).
    fn short_circuit(&mut self, expr: &Expr, emit: impl FnOnce(&mut Self)) {
        if !Self::chain_has_nullsafe(expr) {
            emit(self);
            return;
        }
        self.nullsafe_jumps.push(Vec::new());
        emit(self);
        let end = self.chunk.code.len();
        for jump in self.nullsafe_jumps.pop().unwrap_or_default() {
            self.patch_jump(jump, end);
        }
    }

    fn chain_has_nullsafe(expr: &Expr) -> bool {
        match expr {
            Expr::NullsafePropertyFetch { .. } | Expr::NullsafeMethodCall { .. } => true,
            Expr::PropertyFetch { target, .. } | Expr::MethodCall { target, .. } => {
                Self::chain_has_nullsafe(target)
            }
            Expr::ArrayDimFetch { array, .. } => Self::chain_has_nullsafe(array),
            Expr::StaticCall { class, .. } | Expr::ClassConstFetch { class, .. } => {
                Self::chain_has_nullsafe(class)
            }
            _ => false,
        }
    }

    /// `->name` / `->$name` / `->{expr}` after the object is on the stack.
    fn emit_property_fetch_tail(&mut self, property: &Expr) {
        if let Expr::Variable { span, .. } = property {
            let name = self.get_text(*span);
            if !name.starts_with(b"$") {
                let sym = self.interner.intern(name);
                self.chunk.code.push(OpCode::FetchProp(sym));
            } else {
                // Dynamic property fetch $this->$prop
                self.emit_expr(property);
                self.chunk.code.push(OpCode::FetchPropDynamic);
            }
        } else {
            // Handle dynamic property fetch with expression: $this->{$expr}
            self.emit_expr(property);
            self.chunk.code.push(OpCode::FetchPropDynamic);
        }
    }

    /// `->name(args)` / `->$name(args)` after the object is on the stack.
    fn emit_method_call_tail(&mut self, method: &Expr, args: &[Arg]) {
        if let Expr::Variable { span, .. } = method {
            let name = self.get_text(*span);
            if !name.starts_with(b"$") {
                self.emit_call_args(args);
                let sym = self.interner.intern(name);
                self.chunk
                    .code
                    .push(OpCode::CallMethod(sym, args.len() as u8));
            } else {
                // Dynamic method call: $obj->$method()
                self.emit_expr(method);
                self.emit_call_args(args);
                self.chunk
                    .code
                    .push(OpCode::CallMethodDynamic(args.len() as u8));
            }
        } else {
            // Dynamic method call with expression: $obj->{$expr}()
            self.emit_expr(method);
            self.emit_call_args(args);
            self.chunk
                .code
                .push(OpCode::CallMethodDynamic(args.len() as u8));
        }
    }

    /// Whether `expr` is the variable `$this`.
    fn is_this(&self, expr: &Expr) -> bool {
        matches!(expr, Expr::Variable { span, .. } if self.get_text(*span) == b"$this")
    }

    /// `$this`, or a list() / [] destructuring pattern that assigns to it.
    fn assigns_this(&self, target: &Expr) -> bool {
        match target {
            Expr::Array { items, .. } => items.iter().any(|item| self.assigns_this(item.value)),
            _ => self.is_this(target),
        }
    }

    /// Compile errors of an expression: writing to `$this`, reading `$a[]`.
    fn this_misuse(&self, expr: &Expr) -> Option<&'static str> {
        let target = match expr {
            Expr::Assign { var, .. }
            | Expr::AssignOp { var, .. }
            | Expr::AssignRef { var, .. }
            | Expr::PostInc { var, .. }
            | Expr::PostDec { var, .. } => var,
            Expr::Unary {
                op: UnaryOp::PreInc | UnaryOp::PreDec,
                expr,
                ..
            } => expr,
            Expr::ArrayDimFetch { dim: None, .. } => return Some("Cannot use [] for reading"),
            _ => return None,
        };
        self.assigns_this(target).then_some("Cannot re-assign $this")
    }

    /// `self::`, `parent::` or `static::` in a free function, where PHP
    /// knows at compile time that there is no class (zend_is_scope_known:
    /// not at file level, which an include may run inside a class, and not
    /// in a closure, which may be bound to one).
    fn class_scope_misuse(&self, expr: &Expr) -> Option<String> {
        let class = match expr {
            Expr::New { class, .. } | Expr::StaticCall { class, .. } | Expr::ClassConstFetch { class, .. } => class,
            _ => return None,
        };
        let Expr::Variable { span, .. } = class else { return None };
        let name = self.get_text(*span).to_ascii_lowercase();
        if !matches!(name.as_slice(), b"self" | b"parent" | b"static") {
            return None;
        }
        let function = self.current_function.and_then(|f| self.interner.lookup(f))?;
        if self.current_class.is_some() || self.current_trait.is_some() || function == b"{closure}" {
            return None;
        }
        Some(format!(
            "Cannot use \"{}\" when no class scope is active",
            String::from_utf8_lossy(&name)
        ))
    }

    /// Compile errors of a statement that binds `$this`.
    fn stmt_this_misuse(&self, stmt: &Stmt) -> Option<(usize, &'static str)> {
        match stmt {
            Stmt::Foreach {
                key_var, value_var, ..
            } => key_var
                .iter()
                .chain([value_var])
                .find(|v| self.assigns_this(v))
                .map(|v| (v.span().start, "Cannot re-assign $this")),
            Stmt::Global { vars, .. } => vars
                .iter()
                .find(|v| self.is_this(v))
                .map(|v| (v.span().start, "Cannot use $this as global variable")),
            Stmt::Static { vars, .. } => vars
                .iter()
                .find(|v| self.is_this(v.var))
                .map(|v| (v.span.start, "Cannot use $this as static variable")),
            Stmt::Unset { vars, .. } => vars
                .iter()
                .find(|v| self.is_this(v))
                .map(|v| (v.span().start, "Cannot unset $this")),
            Stmt::Try { catches, .. } => catches
                .iter()
                .find(|c| c.var.is_some_and(|t| self.get_text(t.span) == b"$this"))
                .map(|c| (c.span.start, "Cannot re-assign $this")),
            _ => None,
        }
    }

    /// zend_check_magic_method_implementation: the signature rules of
    /// __get, __set, __toString and the other magic methods.
    fn check_magic_method(
        &mut self,
        class_sym: Symbol,
        name: &Token,
        params: &[crate::parser::ast::Param],
        is_static: bool,
        visibility: Visibility,
        return_type: Option<&Type>,
    ) {
        use magic_types::*;
        let method = self.get_text(name.span);
        if !method.starts_with(b"__") {
            return;
        }
        let lower = method.to_ascii_lowercase();
        // (arguments, static: Some(required), public, argument types, return type)
        let (args, must_be_static, public, arg_types, ret): (Option<usize>, Option<bool>, bool, &[u32], Option<u32>) =
            match lower.as_slice() {
                b"__construct" => (None, Some(false), false, &[], None),
                b"__destruct" => (Some(0), Some(false), false, &[], None),
                b"__clone" => (Some(0), Some(false), false, &[], Some(VOID)),
                b"__get" => (Some(1), Some(false), true, &[STRING], None),
                b"__set" => (Some(2), Some(false), true, &[STRING], Some(VOID)),
                b"__unset" => (Some(1), Some(false), true, &[STRING], Some(VOID)),
                b"__isset" => (Some(1), Some(false), true, &[STRING], Some(BOOL)),
                b"__call" => (Some(2), Some(false), true, &[STRING, ARRAY], None),
                b"__callstatic" => (Some(2), Some(true), true, &[STRING, ARRAY], None),
                b"__tostring" => (Some(0), Some(false), true, &[], Some(STRING)),
                b"__debuginfo" => (Some(0), Some(false), true, &[], Some(ARRAY | NULL)),
                b"__serialize" => (Some(0), Some(false), true, &[], Some(ARRAY)),
                b"__unserialize" => (Some(1), Some(false), true, &[ARRAY], Some(VOID)),
                b"__set_state" => (Some(1), Some(true), true, &[ARRAY], Some(OBJECT)),
                b"__invoke" => (None, Some(false), true, &[], None),
                b"__sleep" => (Some(0), Some(false), true, &[], Some(ARRAY)),
                b"__wakeup" => (Some(0), Some(false), true, &[], Some(VOID)),
                _ => return,
            };
        let class = String::from_utf8_lossy(self.interner.lookup(class_sym).unwrap_or(b"")).into_owned();
        let method = String::from_utf8_lossy(method).into_owned();
        let at = name.span.start;
        if let Some(expected) = args {
            let count = params.iter().filter(|p| !p.variadic).count();
            if count != expected {
                let message = match expected {
                    0 => format!("Method {}::{}() cannot take arguments", class, method),
                    1 => format!("Method {}::{}() must take exactly 1 argument", class, method),
                    n => format!("Method {}::{}() must take exactly {} arguments", class, method, n),
                };
                return self.compile_error(at, message);
            }
            if params.iter().take(expected).any(|p| p.by_ref) {
                let message = format!("Method {}::{}() cannot take arguments by reference", class, method);
                return self.compile_error(at, message);
            }
        }
        match must_be_static {
            Some(false) if is_static => {
                return self.compile_error(at, format!("Method {}::{}() cannot be static", class, method));
            }
            Some(true) if !is_static => {
                return self.compile_error(at, format!("Method {}::{}() must be static", class, method));
            }
            _ => {}
        }
        if public && visibility != Visibility::Public {
            let message = format!("The magic method {}::{}() must have public visibility", class, method);
            self.compile_warning(ErrorLevel::Warning, at, message);
        }
        for (i, &allowed) in arg_types.iter().enumerate() {
            let Some(param) = params.get(i) else { break };
            if let Some(ty) = param.ty
                && self.type_mask(ty).0 & allowed == 0
            {
                let message = format!(
                    "{}::{}(): Parameter #{} ({}) must be of type {} when declared",
                    class,
                    method,
                    i + 1,
                    String::from_utf8_lossy(self.get_text(param.name.span)),
                    mask_name(allowed)
                );
                return self.compile_error(at, message);
            }
        }
        if lower == b"__construct" || lower == b"__destruct" {
            if return_type.is_some() {
                let message = format!("Method {}::{}() cannot declare a return type", class, method);
                self.compile_error(at, message);
            }
            return;
        }
        if let (Some(allowed), Some(ty)) = (ret, return_type) {
            let (mask, mut complex) = self.type_mask(ty);
            if mask & NEVER != 0 {
                return;
            }
            let mut extra = mask & !allowed;
            if extra & STATIC != 0 {
                extra &= !STATIC;
                complex = true;
            }
            if extra != 0 || (complex && allowed != OBJECT) {
                let message = format!(
                    "{}::{}(): Return type must be {} when declared",
                    class,
                    method,
                    mask_name(allowed)
                );
                self.compile_error(at, message);
            }
        }
    }

    /// A declared type as zend type bits, and whether it names classes.
    fn type_mask(&self, ty: &Type) -> (u32, bool) {
        use magic_types::*;
        let single = match ty {
            Type::Simple(token) => Some(token.span),
            Type::Name(name) if name.parts.len() == 1 => Some(name.parts[0].span),
            _ => None,
        };
        if let Some(span) = single {
            return match self.get_text(span).to_ascii_lowercase().as_slice() {
                b"null" => (NULL, false),
                b"false" => (FALSE, false),
                b"true" => (TRUE, false),
                b"bool" => (BOOL, false),
                b"int" => (INT, false),
                b"float" => (FLOAT, false),
                b"string" => (STRING, false),
                b"array" => (ARRAY, false),
                b"object" => (OBJECT, false),
                b"void" => (VOID, false),
                b"never" => (NEVER, false),
                b"static" => (STATIC, false),
                b"mixed" => (ANY, false),
                b"iterable" => (ARRAY, true),
                b"callable" => (CALLABLE, false),
                _ => (0, true),
            };
        }
        match ty {
            Type::Simple(_) | Type::Name(_) | Type::Intersection(_) => (0, true),
            Type::Nullable(inner) => {
                let (mask, complex) = self.type_mask(inner);
                (mask | NULL, complex)
            }
            Type::Union(types) => types.iter().fold((0, false), |(mask, complex), t| {
                let (m, c) = self.type_mask(t);
                (mask | m, complex || c)
            }),
        }
    }

    /// PHP rejects a `?->` chain as something to write to or reference.
    fn nullsafe_misuse(expr: &Expr) -> Option<&'static str> {
        let target = match expr {
            Expr::Assign { var, .. }
            | Expr::AssignOp { var, .. }
            | Expr::PostInc { var, .. }
            | Expr::PostDec { var, .. } => var,
            Expr::Unary {
                op: UnaryOp::PreInc | UnaryOp::PreDec,
                expr,
                ..
            } => expr,
            Expr::AssignRef { var, expr, .. } => {
                if Self::chain_has_nullsafe(var) {
                    return Some("Can't use nullsafe operator in write context");
                }
                return Self::chain_has_nullsafe(expr)
                    .then_some("Cannot take reference of a nullsafe chain");
            }
            _ => return None,
        };
        Self::chain_has_nullsafe(target).then_some("Can't use nullsafe operator in write context")
    }

    /// `?->`: the base is on the stack; null ends the chain.
    fn emit_nullsafe_check(&mut self) {
        let jump = self.chunk.code.len();
        self.chunk.code.push(OpCode::JmpNull(0));
        if let Some(chain) = self.nullsafe_jumps.last_mut() {
            chain.push(jump);
        }
    }

    fn emit_expr_inner(&mut self, expr: &Expr) {
        let append_fetch_ok = std::mem::take(&mut self.append_fetch_ok);
        if let Some(message) = Self::nullsafe_misuse(expr) {
            self.compile_error(expr.span().start, message);
            return;
        }
        if let Some(message) = self.this_misuse(expr)
            && !(append_fetch_ok && matches!(expr, Expr::ArrayDimFetch { dim: None, .. }))
        {
            self.compile_error(expr.span().start, message);
            return;
        }
        if let Some(message) = self.class_scope_misuse(expr) {
            self.compile_error(expr.span().start, message);
            return;
        }
        match expr {
            Expr::Integer { value, .. } => {
                let idx = self.add_constant(integer_literal(value));
                self.chunk.code.push(OpCode::Const(idx as u16));
            }
            Expr::Float { value, .. } => {
                let f = float_literal(value);
                let idx = self.add_constant(Val::Float(f));
                self.chunk.code.push(OpCode::Const(idx as u16));
            }
            Expr::String { value, .. } => {
                let s = string_literal(value);
                let idx = self.add_constant(Val::String(s.into()));
                self.chunk.code.push(OpCode::Const(idx as u16));
            }
            Expr::InterpolatedString { parts, .. } => {
                if parts.is_empty() {
                    let idx = self.add_constant(Val::String(Vec::<u8>::new().into()));
                    self.chunk.code.push(OpCode::Const(idx as u16));
                } else {
                    for (i, part) in parts.iter().enumerate() {
                        self.emit_expr(*part);
                        if i > 0 {
                            self.chunk.code.push(OpCode::Concat);
                        }
                    }
                }
            }
            Expr::Boolean { value, .. } => {
                let idx = self.add_constant(Val::Bool(*value));
                self.chunk.code.push(OpCode::Const(idx as u16));
            }
            Expr::Null { .. } => {
                let idx = self.add_constant(Val::Null);
                self.chunk.code.push(OpCode::Const(idx as u16));
            }
            Expr::Binary {
                left, op, right, ..
            } => {
                match op {
                    BinaryOp::And | BinaryOp::LogicalAnd => {
                        self.emit_expr(left);
                        let end_jump = self.chunk.code.len();
                        self.chunk.code.push(OpCode::JmpZEx(0));
                        self.emit_expr(right);
                        let end_label = self.chunk.code.len();
                        self.chunk.code[end_jump] = OpCode::JmpZEx(end_label as u32);
                        self.chunk.code.push(OpCode::Cast(1)); // Bool
                    }
                    BinaryOp::Or | BinaryOp::LogicalOr => {
                        self.emit_expr(left);
                        let end_jump = self.chunk.code.len();
                        self.chunk.code.push(OpCode::JmpNzEx(0));
                        self.emit_expr(right);
                        let end_label = self.chunk.code.len();
                        self.chunk.code[end_jump] = OpCode::JmpNzEx(end_label as u32);
                        self.chunk.code.push(OpCode::Cast(1)); // Bool
                    }
                    BinaryOp::Coalesce => {
                        self.emit_expr_quiet(left);
                        let end_jump = self.chunk.code.len();
                        self.chunk.code.push(OpCode::Coalesce(0));
                        self.emit_expr(right);
                        let end_label = self.chunk.code.len();
                        self.chunk.code[end_jump] = OpCode::Coalesce(end_label as u32);
                    }
                    BinaryOp::Instanceof => {
                        // For instanceof, the class name should be treated as a literal string,
                        // not a constant lookup. PHP allows bare identifiers like "instanceof Foo".
                        self.emit_expr(left);

                        // Special handling for bare class names
                        match right {
                            Expr::Variable { span, .. } => {
                                // Bare identifier - treat as class name string
                                let name = self.get_text(*span);
                                let class_name_str = if name.starts_with(b"$") {
                                    // It's actually a variable, evaluate it normally
                                    self.emit_expr(right);
                                    self.chunk.code.push(OpCode::InstanceOf);
                                    return;
                                } else {
                                    // Bare class name - push as string constant
                                    Val::String(self.resolve_class_name(self.raw_text(*span)).into())
                                };
                                let const_idx = self.add_constant(class_name_str) as u16;
                                self.chunk.code.push(OpCode::Const(const_idx));
                            }
                            _ => {
                                // Complex expression - evaluate normally
                                self.emit_expr(right);
                            }
                        }

                        self.chunk.code.push(OpCode::InstanceOf);
                    }
                    _ => {
                        self.emit_expr(left);
                        self.emit_expr(right);
                        match op {
                            BinaryOp::Plus => self.chunk.code.push(OpCode::Add),
                            BinaryOp::Minus => self.chunk.code.push(OpCode::Sub),
                            BinaryOp::Mul => self.chunk.code.push(OpCode::Mul),
                            BinaryOp::Div => self.chunk.code.push(OpCode::Div),
                            BinaryOp::Mod => self.chunk.code.push(OpCode::Mod),
                            BinaryOp::Concat => self.chunk.code.push(OpCode::Concat),
                            BinaryOp::Pow => self.chunk.code.push(OpCode::Pow),
                            BinaryOp::BitAnd => self.chunk.code.push(OpCode::BitwiseAnd),
                            BinaryOp::BitOr => self.chunk.code.push(OpCode::BitwiseOr),
                            BinaryOp::BitXor => self.chunk.code.push(OpCode::BitwiseXor),
                            BinaryOp::ShiftLeft => self.chunk.code.push(OpCode::ShiftLeft),
                            BinaryOp::ShiftRight => self.chunk.code.push(OpCode::ShiftRight),
                            BinaryOp::EqEq => self.chunk.code.push(OpCode::IsEqual),
                            BinaryOp::EqEqEq => self.chunk.code.push(OpCode::IsIdentical),
                            BinaryOp::NotEq => self.chunk.code.push(OpCode::IsNotEqual),
                            BinaryOp::NotEqEq => self.chunk.code.push(OpCode::IsNotIdentical),
                            BinaryOp::Gt => self.chunk.code.push(OpCode::IsGreater),
                            BinaryOp::Lt => self.chunk.code.push(OpCode::IsLess),
                            BinaryOp::GtEq => self.chunk.code.push(OpCode::IsGreaterOrEqual),
                            BinaryOp::LtEq => self.chunk.code.push(OpCode::IsLessOrEqual),
                            BinaryOp::Spaceship => self.chunk.code.push(OpCode::Spaceship),
                            BinaryOp::LogicalXor => self.chunk.code.push(OpCode::BoolXor),
                            // Instanceof is handled above
                            BinaryOp::Instanceof => {}
                            _ => {}
                        }
                    }
                }
            }
            Expr::Match {
                condition, arms, ..
            } => {
                self.emit_expr(condition);

                let mut end_jumps = Vec::new();

                for arm in *arms {
                    if let Some(conds) = arm.conditions {
                        let mut body_jump_indices = Vec::new();

                        for cond in conds {
                            self.chunk.code.push(OpCode::Dup);
                            self.emit_expr(cond);
                            self.chunk.code.push(OpCode::IsIdentical); // Strict

                            let jump_idx = self.chunk.code.len();
                            self.chunk.code.push(OpCode::JmpIfTrue(0)); // Jump to body
                            body_jump_indices.push(jump_idx);
                        }

                        // If we are here, none matched. Jump to next arm.
                        let skip_body_idx = self.chunk.code.len();
                        self.chunk.code.push(OpCode::Jmp(0));

                        // Body start
                        let body_start = self.chunk.code.len();
                        for idx in body_jump_indices {
                            self.patch_jump(idx, body_start);
                        }

                        // Pop condition before body
                        self.chunk.code.push(OpCode::Pop);
                        self.emit_expr(arm.body);

                        // Jump to end
                        end_jumps.push(self.chunk.code.len());
                        self.chunk.code.push(OpCode::Jmp(0));

                        // Patch skip_body_idx to here (next arm)
                        self.patch_jump(skip_body_idx, self.chunk.code.len());
                    } else {
                        // Default arm
                        self.chunk.code.push(OpCode::Pop); // Pop condition
                        self.emit_expr(arm.body);
                        end_jumps.push(self.chunk.code.len());
                        self.chunk.code.push(OpCode::Jmp(0));
                    }
                }

                // No match found
                self.chunk.code.push(OpCode::MatchError);

                let end_label = self.chunk.code.len();
                for idx in end_jumps {
                    self.patch_jump(idx, end_label);
                }
            }
            Expr::Print { expr, .. } => {
                self.emit_expr(expr);
                self.chunk.code.push(OpCode::Echo);
                let idx = self.add_constant(Val::Int(1));
                self.chunk.code.push(OpCode::Const(idx as u16));
            }
            Expr::Include { expr, kind, .. } => {
                self.emit_expr(expr);
                let include_type = match kind {
                    IncludeKind::Include => 2,
                    IncludeKind::IncludeOnce => 3,
                    IncludeKind::Require => 4,
                    IncludeKind::RequireOnce => 5,
                };
                let idx = self.add_constant(Val::Int(include_type));
                self.chunk.code.push(OpCode::Const(idx as u16));
                self.chunk.code.push(OpCode::IncludeOrEval);
            }
            Expr::Unary { op, expr, .. } => {
                match op {
                    UnaryOp::Reference => {
                        // Handle &$var
                        if let Expr::Variable { span, .. } = expr {
                            let name = self.get_text(*span);
                            if name.starts_with(b"$") {
                                let var_name = &name[1..];
                                let sym = self.interner.intern(var_name);
                                self.chunk.code.push(OpCode::MakeVarRef(sym));
                            }
                        } else {
                            // Reference to something else?
                            self.emit_expr(expr);
                            self.chunk.code.push(OpCode::MakeRef);
                        }
                    }
                    UnaryOp::Minus | UnaryOp::Plus => {
                        // expr * -1 / expr * 1, as Zend compiles them (keeps -0.0,
                        // converts numeric strings, operand order in errors)
                        self.emit_expr(expr);
                        let factor = if matches!(op, UnaryOp::Minus) { -1 } else { 1 };
                        let idx = self.add_constant(Val::Int(factor));
                        self.chunk.code.push(OpCode::Const(idx as u16));
                        self.chunk.code.push(OpCode::Mul);
                    }
                    UnaryOp::Not => {
                        self.emit_expr(expr);
                        self.chunk.code.push(OpCode::BoolNot);
                    }
                    UnaryOp::BitNot => {
                        self.emit_expr(expr);
                        self.chunk.code.push(OpCode::BitwiseNot);
                    }
                    UnaryOp::PreInc => {
                        match expr {
                            Expr::Variable { span, .. } => {
                                let name = self.get_text(*span);
                                if name.starts_with(b"$") {
                                    let sym = self.interner.intern(&name[1..]);
                                    self.chunk.code.push(OpCode::FetchRw(sym));
                                    self.chunk.code.push(OpCode::PreInc);
                                }
                            }
                            Expr::PropertyFetch {
                                target, property, ..
                            } => {
                                // ++$obj->prop
                                self.emit_write_base(target);
                                // Property name (could be identifier or expression)
                                let prop_name = self.get_text(property.span());
                                let const_idx =
                                    self.add_constant(Val::String(Rc::new(prop_name.to_vec())));
                                self.chunk.code.push(OpCode::Const(const_idx as u16));
                                self.chunk.code.push(OpCode::PreIncObj);
                            }
                            Expr::ClassConstFetch {
                                class, constant, ..
                            } => {
                                // ++Class::$property
                                if self.emit_static_property_access(class, constant) {
                                    self.chunk.code.push(OpCode::PreIncStaticProp);
                                } else {
                                    self.emit_expr(expr);
                                }
                            }
                            _ => {
                                if matches!(expr, Expr::ArrayDimFetch { .. }) {
                                    // $a[k]++: the element's slot, created if missing.
                                    self.emit_expr_for_update(expr);
                                    self.chunk.code.push(OpCode::PreInc);
                                } else {
                                    self.emit_expr(expr);
                                }
                            }
                        }
                    }
                    UnaryOp::PreDec => {
                        match expr {
                            Expr::Variable { span, .. } => {
                                let name = self.get_text(*span);
                                if name.starts_with(b"$") {
                                    let sym = self.interner.intern(&name[1..]);
                                    self.chunk.code.push(OpCode::FetchRw(sym));
                                    self.chunk.code.push(OpCode::PreDec);
                                }
                            }
                            Expr::PropertyFetch {
                                target, property, ..
                            } => {
                                // --$obj->prop
                                self.emit_write_base(target);
                                let prop_name = self.get_text(property.span());
                                let const_idx =
                                    self.add_constant(Val::String(Rc::new(prop_name.to_vec())));
                                self.chunk.code.push(OpCode::Const(const_idx as u16));
                                self.chunk.code.push(OpCode::PreDecObj);
                            }
                            Expr::ClassConstFetch {
                                class, constant, ..
                            } => {
                                // --Class::$property
                                if self.emit_static_property_access(class, constant) {
                                    self.chunk.code.push(OpCode::PreDecStaticProp);
                                } else {
                                    self.emit_expr(expr);
                                }
                            }
                            _ => {
                                if matches!(expr, Expr::ArrayDimFetch { .. }) {
                                    // $a[k]++: the element's slot, created if missing.
                                    self.emit_expr_for_update(expr);
                                    self.chunk.code.push(OpCode::PreDec);
                                } else {
                                    self.emit_expr(expr);
                                }
                            }
                        }
                    }
                    UnaryOp::ErrorSuppress => {
                        // @ operator: suppress errors for the expression
                        self.chunk.code.push(OpCode::BeginSilence);
                        self.emit_expr(expr);
                        self.chunk.code.push(OpCode::EndSilence);
                    }
                }
            }
            Expr::PostInc { var, .. } => {
                match var {
                    Expr::Variable { span, .. } => {
                        let name = self.get_text(*span);
                        if name.starts_with(b"$") {
                            let sym = self.interner.intern(&name[1..]);
                            self.chunk.code.push(OpCode::FetchRw(sym));
                            self.chunk.code.push(OpCode::PostInc);
                        }
                    }
                    Expr::PropertyFetch {
                        target, property, ..
                    } => {
                        // $obj->prop++
                        self.emit_write_base(target);
                        let prop_name = self.get_text(property.span());
                        let const_idx = self.add_constant(Val::String(Rc::new(prop_name.to_vec())));
                        self.chunk.code.push(OpCode::Const(const_idx as u16));
                        self.chunk.code.push(OpCode::PostIncObj);
                    }
                    Expr::ClassConstFetch {
                        class, constant, ..
                    } => {
                        // Class::$property++
                        if self.emit_static_property_access(class, constant) {
                            self.chunk.code.push(OpCode::PostIncStaticProp);
                        } else {
                            self.emit_expr(var);
                        }
                    }
                    _ => {
                        if matches!(var, Expr::ArrayDimFetch { .. }) {
                            // $a[k]++: the element's slot, created if missing.
                            self.emit_expr_for_update(var);
                            self.chunk.code.push(OpCode::PostInc);
                        } else {
                            self.emit_expr(var);
                        }
                    }
                }
            }
            Expr::PostDec { var, .. } => {
                match var {
                    Expr::Variable { span, .. } => {
                        let name = self.get_text(*span);
                        if name.starts_with(b"$") {
                            let sym = self.interner.intern(&name[1..]);
                            self.chunk.code.push(OpCode::FetchRw(sym));
                            self.chunk.code.push(OpCode::PostDec);
                        }
                    }
                    Expr::PropertyFetch {
                        target, property, ..
                    } => {
                        // $obj->prop--
                        self.emit_write_base(target);
                        let prop_name = self.get_text(property.span());
                        let const_idx = self.add_constant(Val::String(Rc::new(prop_name.to_vec())));
                        self.chunk.code.push(OpCode::Const(const_idx as u16));
                        self.chunk.code.push(OpCode::PostDecObj);
                    }
                    Expr::ClassConstFetch {
                        class, constant, ..
                    } => {
                        // Class::$property--
                        if self.emit_static_property_access(class, constant) {
                            self.chunk.code.push(OpCode::PostDecStaticProp);
                        } else {
                            self.emit_expr(var);
                        }
                    }
                    _ => {
                        if matches!(var, Expr::ArrayDimFetch { .. }) {
                            // $a[k]++: the element's slot, created if missing.
                            self.emit_expr_for_update(var);
                            self.chunk.code.push(OpCode::PostDec);
                        } else {
                            self.emit_expr(var);
                        }
                    }
                }
            }
            Expr::Ternary {
                condition,
                if_true,
                if_false,
                ..
            } => {
                self.emit_expr(condition);
                if let Some(true_expr) = if_true {
                    // cond ? true : false
                    let else_jump = self.chunk.code.len();
                    self.chunk.code.push(OpCode::JmpIfFalse(0)); // Placeholder

                    self.emit_expr(true_expr);
                    let end_jump = self.chunk.code.len();
                    self.chunk.code.push(OpCode::Jmp(0)); // Placeholder

                    let else_label = self.chunk.code.len();
                    self.chunk.code[else_jump] = OpCode::JmpIfFalse(else_label as u32);

                    self.emit_expr(if_false);
                    let end_label = self.chunk.code.len();
                    self.chunk.code[end_jump] = OpCode::Jmp(end_label as u32);
                } else {
                    // cond ?: false (Elvis)
                    let end_jump = self.chunk.code.len();
                    self.chunk.code.push(OpCode::JmpNzEx(0)); // Placeholder
                    self.emit_expr(if_false);

                    let end_label = self.chunk.code.len();
                    self.chunk.code[end_jump] = OpCode::JmpNzEx(end_label as u32);
                }
            }
            Expr::Cast { kind, expr, .. } => {
                self.emit_expr(expr);
                // Map CastKind to OpCode::Cast(u8)
                // 0=Int, 1=Bool, 2=Float, 3=String, 4=Array, 5=Object, 6=Unset
                let cast_op = match kind {
                    CastKind::Int => 0,
                    CastKind::Bool => 1,
                    CastKind::Float => 2,
                    CastKind::String => 3,
                    CastKind::Array => 4,
                    CastKind::Object => 5,
                    CastKind::Unset => 6,
                    _ => 0, // TODO
                };
                self.chunk.code.push(OpCode::Cast(cast_op));
            }
            Expr::Clone { expr, .. } => {
                self.emit_expr(expr);
                self.chunk.code.push(OpCode::Clone);
            }
            Expr::Exit { expr, .. } | Expr::Die { expr, .. } => {
                if let Some(e) = expr {
                    self.emit_expr(e);
                } else {
                    let idx = self.add_constant(Val::Null);
                    self.chunk.code.push(OpCode::Const(idx as u16));
                }
                self.chunk.code.push(OpCode::Exit);
            }
            Expr::Isset { vars, .. } => {
                if vars.is_empty() {
                    let idx = self.add_constant(Val::Bool(false));
                    self.chunk.code.push(OpCode::Const(idx as u16));
                } else {
                    let mut end_jumps = Vec::new();

                    for (i, var) in vars.iter().enumerate() {
                        match var {
                            Expr::Variable { span, .. } => {
                                let name = self.get_text(*span);
                                if name.starts_with(b"$") {
                                    let sym = self.interner.intern(&name[1..]);
                                    self.chunk.code.push(OpCode::IssetVar(sym));
                                } else {
                                    let idx = self.add_constant(Val::Bool(false));
                                    self.chunk.code.push(OpCode::Const(idx as u16));
                                }
                            }
                            Expr::IndirectVariable { name, .. } => {
                                self.emit_expr(name);
                                self.chunk.code.push(OpCode::IssetVarDynamic);
                            }
                            Expr::ArrayDimFetch { array, dim, span } => {
                                self.emit_expr_quiet(array);
                                if let Some(d) = dim {
                                    self.emit_expr(d);
                                    self.chunk.code.push(OpCode::IssetDim);
                                } else {
                                    self.compile_error(span.start, "Cannot use [] for reading");
                                }
                            }
                            Expr::PropertyFetch {
                                target, property, ..
                            } => {
                                self.emit_expr_quiet(target);
                                if let Expr::Variable { span, .. } = property {
                                    let name = self.get_text(*span);
                                    let sym = self.interner.intern(name);
                                    self.chunk.code.push(OpCode::IssetProp(sym));
                                } else {
                                    self.chunk.code.push(OpCode::Pop);
                                    let idx = self.add_constant(Val::Bool(false));
                                    self.chunk.code.push(OpCode::Const(idx as u16));
                                }
                            }
                            Expr::ClassConstFetch {
                                class, constant, ..
                            } => {
                                let is_static_prop = if let Expr::Variable { span, .. } = constant {
                                    let name = self.get_text(*span);
                                    name.starts_with(b"$")
                                } else {
                                    false
                                };

                                if is_static_prop {
                                    if let Expr::Variable { span, .. } = class {
                                        let name = &self.resolve_class_name(self.raw_text(*span));
                                        if !name.starts_with(b"$") {
                                            let idx = self
                                                .add_constant(Val::String(name.to_vec().into()));
                                            self.chunk.code.push(OpCode::Const(idx as u16));
                                        } else {
                                            let sym = self.interner.intern(&name[1..]);
                                            self.chunk.code.push(OpCode::LoadVar(sym));
                                        }

                                        if let Expr::Variable {
                                            span: prop_span, ..
                                        } = constant
                                        {
                                            let prop_name = self.get_text(*prop_span);
                                            let prop_sym = self.interner.intern(&prop_name[1..]);
                                            self.chunk.code.push(OpCode::IssetStaticProp(prop_sym));
                                        }
                                    } else {
                                        let idx = self.add_constant(Val::Bool(false));
                                        self.chunk.code.push(OpCode::Const(idx as u16));
                                    }
                                } else {
                                    let idx = self.add_constant(Val::Bool(false));
                                    self.chunk.code.push(OpCode::Const(idx as u16));
                                }
                            }
                            _ => {
                                let idx = self.add_constant(Val::Bool(false));
                                self.chunk.code.push(OpCode::Const(idx as u16));
                            }
                        }

                        if i < vars.len() - 1 {
                            self.chunk.code.push(OpCode::Dup);
                            let jump_idx = self.chunk.code.len();
                            self.chunk.code.push(OpCode::JmpIfFalse(0));
                            self.chunk.code.push(OpCode::Pop);
                            end_jumps.push(jump_idx);
                        }
                    }

                    let end_label = self.chunk.code.len();
                    for idx in end_jumps {
                        self.patch_jump(idx, end_label);
                    }
                }
            }
            Expr::Empty { expr, .. } => {
                match expr {
                    Expr::Variable { span, .. } => {
                        let name = self.get_text(*span);
                        if name.starts_with(b"$") {
                            let sym = self.interner.intern(&name[1..]);
                            self.chunk.code.push(OpCode::IssetVar(sym));
                        } else {
                            let idx = self.add_constant(Val::Bool(false));
                            self.chunk.code.push(OpCode::Const(idx as u16));
                        }
                    }
                    Expr::ArrayDimFetch { array, dim, .. } => {
                        self.emit_expr_quiet(array);
                        if let Some(d) = dim {
                            self.emit_expr(d);
                            self.chunk.code.push(OpCode::IssetDim);
                        } else {
                            let idx = self.add_constant(Val::Bool(false));
                            self.chunk.code.push(OpCode::Const(idx as u16));
                        }
                    }
                    Expr::PropertyFetch {
                        target, property, ..
                    } => {
                        self.emit_expr_quiet(target);
                        if let Expr::Variable { span, .. } = property {
                            let name = self.get_text(*span);
                            let sym = self.interner.intern(name);
                            self.chunk.code.push(OpCode::IssetProp(sym));
                        } else {
                            self.chunk.code.push(OpCode::Pop);
                            let idx = self.add_constant(Val::Bool(false));
                            self.chunk.code.push(OpCode::Const(idx as u16));
                        }
                    }
                    Expr::ClassConstFetch {
                        class, constant, ..
                    } => {
                        let is_static_prop = if let Expr::Variable { span, .. } = constant {
                            let name = self.get_text(*span);
                            name.starts_with(b"$")
                        } else {
                            false
                        };

                        if is_static_prop {
                            if let Expr::Variable { span, .. } = class {
                                let name = &self.resolve_class_name(self.raw_text(*span));
                                if !name.starts_with(b"$") {
                                    let idx = self.add_constant(Val::String(name.to_vec().into()));
                                    self.chunk.code.push(OpCode::Const(idx as u16));
                                } else {
                                    let sym = self.interner.intern(&name[1..]);
                                    self.chunk.code.push(OpCode::LoadVar(sym));
                                }

                                if let Expr::Variable {
                                    span: prop_span, ..
                                } = constant
                                {
                                    let prop_name = self.get_text(*prop_span);
                                    let prop_sym = self.interner.intern(&prop_name[1..]);
                                    self.chunk.code.push(OpCode::IssetStaticProp(prop_sym));
                                }
                            } else {
                                let idx = self.add_constant(Val::Bool(false));
                                self.chunk.code.push(OpCode::Const(idx as u16));
                            }
                        } else {
                            let idx = self.add_constant(Val::Bool(false));
                            self.chunk.code.push(OpCode::Const(idx as u16));
                        }
                    }
                    _ => {
                        self.emit_expr(expr);
                        self.chunk.code.push(OpCode::BoolNot);
                        return;
                    }
                }

                let jump_if_not_set = self.chunk.code.len();
                self.chunk.code.push(OpCode::JmpIfFalse(0));

                self.emit_expr(expr);
                self.chunk.code.push(OpCode::BoolNot);

                let jump_end = self.chunk.code.len();
                self.chunk.code.push(OpCode::Jmp(0));

                let label_true = self.chunk.code.len();
                self.patch_jump(jump_if_not_set, label_true);

                let idx = self.add_constant(Val::Bool(true));
                self.chunk.code.push(OpCode::Const(idx as u16));

                let label_end = self.chunk.code.len();
                self.patch_jump(jump_end, label_end);
            }
            Expr::Eval { expr, .. } => {
                self.emit_expr(expr);
                // Emit ZEND_EVAL (type=1) for eval()
                let idx = self.add_constant(Val::Int(1));
                self.chunk.code.push(OpCode::Const(idx as u16));
                self.chunk.code.push(OpCode::IncludeOrEval);
            }
            Expr::Yield {
                key, value, from, ..
            } => {
                self.is_generator = true;
                if *from {
                    if let Some(v) = value {
                        self.emit_expr(v);
                    } else {
                        let idx = self.add_constant(Val::Null);
                        self.chunk.code.push(OpCode::Const(idx as u16));
                    }
                    self.chunk.code.push(OpCode::YieldFrom);
                } else {
                    let has_key = key.is_some();
                    if let Some(k) = key {
                        self.emit_expr(k);
                    }

                    if let Some(v) = value {
                        self.emit_expr(v);
                    } else {
                        let idx = self.add_constant(Val::Null);
                        self.chunk.code.push(OpCode::Const(idx as u16));
                    }
                    self.chunk.code.push(OpCode::Yield(has_key));
                    self.chunk.code.push(OpCode::GetSentValue);
                }
            }
            Expr::Closure {
                params,
                uses,
                body,
                by_ref,
                is_static,
                return_type,
                span,
                ..
            } => {
                // 1. Collect param info
                struct ParamInfo<'a> {
                    name_span: crate::parser::span::Span,
                    by_ref: bool,
                    ty: Option<&'a Type<'a>>,
                    default: Option<&'a Expr<'a>>,
                    variadic: bool,
                    attributes: &'a [AttributeGroup<'a>],
                }

                let mut param_infos = Vec::new();
                for param in *params {
                    param_infos.push(ParamInfo {
                        name_span: param.name.span,
                        by_ref: param.by_ref,
                        ty: param.ty,
                        default: param.default.as_ref().map(|e| *e),
                        variadic: param.variadic,
                        attributes: param.attributes,
                    });
                }

                // 2. Create emitter with inherited context (closures inherit context)
                let closure_sym = self.interner.intern(b"{closure}");
                let (child_line, child_line_starts) = self.child_lines(span.start);
                    let compile_error = self.compile_error.clone();
                    let mut func_emitter = Emitter::new(self.source, self.interner);
                    func_emitter.compile_error = compile_error;
                    func_emitter.compile_warnings = self.compile_warnings.clone();
                    func_emitter.imports = self.imports.clone();
                    func_emitter.current_line = child_line;
                    func_emitter.line_starts = child_line_starts;
                func_emitter.file_path = self.file_path.clone();
                func_emitter.current_class = self.current_class;
                func_emitter.current_function = Some(closure_sym);
                func_emitter.current_namespace = self.current_namespace;
                func_emitter.chunk.strict_types = self.chunk.strict_types;

                // 3. Process params
                let mut param_syms = Vec::new();
                for (i, info) in param_infos.iter().enumerate() {
                    let p_name = func_emitter.get_text(info.name_span);
                    if p_name == b"$this" {
                        func_emitter.compile_error(info.name_span.start, "Cannot use $this as parameter");
                    }
                    if p_name.starts_with(b"$") {
                        let sym = func_emitter.interner.intern(&p_name[1..]);
                        let param_type = info.ty.and_then(|ty| func_emitter.convert_type(ty));
                        let default_value = info.default.map(|expr| func_emitter.const_initializer(expr));

                        param_syms.push(FuncParam {
                            name: sym,
                            by_ref: info.by_ref,
                            param_type,
                            is_variadic: info.variadic,
                            default_value: default_value.clone(),
                            default_display: info.default.map(|e| func_emitter.default_display(e).into()),
                            attributes: func_emitter.build_attribute_instances(info.attributes),
                        });

                        if let Some(val) = default_value.clone() {
                            let val = val;
                            let idx = func_emitter.add_constant(val);
                            func_emitter
                                .chunk
                                .code
                                .push(OpCode::RecvInit(i as u32, idx as u16));
                        } else {
                            func_emitter.chunk.code.push(if info.variadic {
                                OpCode::RecvVariadic(i as u32)
                            } else {
                                OpCode::Recv(i as u32)
                            });
                        }
                    }
                }

                let (mut func_chunk, is_generator) = func_emitter.compile(body);
                func_chunk.returns_ref = *by_ref;

                // Extract uses
                let mut use_syms = Vec::new();
                for use_var in *uses {
                    let u_name = self.get_text(use_var.var.span);
                    if u_name == b"$this" {
                        self.compile_error(use_var.var.span.start, "Cannot use $this as lexical variable");
                    }
                    if u_name.starts_with(b"$") {
                        let sym = self.interner.intern(&u_name[1..]);
                        use_syms.push(sym);

                        if use_var.by_ref {
                            self.chunk.code.push(OpCode::LoadRef(sym));
                        } else {
                            // Emit code to push the captured variable onto the stack
                            self.chunk.code.push(OpCode::LoadVar(sym));
                            self.chunk.code.push(OpCode::Copy);
                        }
                    }
                }

                // Convert return type
                let ret_type = return_type.and_then(|rt| self.convert_type(rt));

                let user_func = UserFunc {
                    params: param_syms,
                    uses: use_syms.clone(),
                    chunk: Rc::new(func_chunk),
                    is_static: *is_static,
                    is_generator,
                    statics: Rc::new(RefCell::new(HashMap::new())),
                    return_type: ret_type,
                    attributes: Vec::new(),
                    decl_line: self.line_at(span.start),
                    is_final: false,
                };

                let func_res = Val::Resource(Rc::new(user_func));
                let const_idx = self.add_constant(func_res);

                self.chunk
                    .code
                    .push(OpCode::Closure(const_idx as u32, use_syms.len() as u32));
            }
            Expr::ArrowFunction {
                params,
                expr,
                by_ref,
                is_static,
                return_type,
                span,
                ..
            } => {
                let closure_sym = self.interner.intern(b"{closure}");
                let (child_line, child_line_starts) = self.child_lines(span.start);
                let compile_error = self.compile_error.clone();
                let mut func_emitter = Emitter::new(self.source, self.interner);
                func_emitter.compile_error = compile_error;
                func_emitter.compile_warnings = self.compile_warnings.clone();
                func_emitter.imports = self.imports.clone();
                func_emitter.current_line = child_line;
                func_emitter.line_starts = child_line_starts;
                func_emitter.file_path = self.file_path.clone();
                func_emitter.current_class = self.current_class;
                func_emitter.current_function = Some(closure_sym);
                func_emitter.current_namespace = self.current_namespace;
                func_emitter.chunk.strict_types = self.chunk.strict_types;
                func_emitter.inherited_strict_types = Some(self.chunk.strict_types);

                let mut param_syms = Vec::new();
                for (i, param) in params.iter().enumerate() {
                    let p_name = func_emitter.get_text(param.name.span);
                    if p_name == b"$this" {
                        func_emitter.compile_error(param.name.span.start, "Cannot use $this as parameter");
                    }
                    let Some(p_name) = p_name.strip_prefix(b"$") else {
                        continue;
                    };
                    let sym = func_emitter.interner.intern(p_name);
                    let param_type = param.ty.and_then(|ty| func_emitter.convert_type(ty));
                    let default_value = param.default.map(|e| func_emitter.const_initializer(e));
                    param_syms.push(FuncParam {
                        name: sym,
                        by_ref: param.by_ref,
                        param_type,
                        is_variadic: param.variadic,
                        default_value: default_value.clone(),
                        default_display: param.default.map(|e| func_emitter.default_display(e).into()),
                        attributes: func_emitter.build_attribute_instances(param.attributes),
                    });
                    match default_value {
                        Some(val) => {
                            let idx = func_emitter.add_constant(val);
                            func_emitter.chunk.code.push(OpCode::RecvInit(i as u32, idx as u16));
                        }
                        None if param.variadic => {
                            func_emitter.chunk.code.push(OpCode::RecvVariadic(i as u32))
                        }
                        None => func_emitter.chunk.code.push(OpCode::Recv(i as u32)),
                    }
                }

                let (mut func_chunk, is_generator) = func_emitter.compile_arrow_body(expr);
                func_chunk.returns_ref = *by_ref;

                // Every variable the body names is captured by value, except
                // the parameters and $this (zend_compile.c: find_implicit_binds).
                let mut use_syms: Vec<Symbol> = Vec::new();
                for name in self.variables_in(expr.span()) {
                    let sym = self.interner.intern(&name);
                    if name == b"this" || param_syms.iter().any(|p| p.name == sym) || use_syms.contains(&sym) {
                        continue;
                    }
                    use_syms.push(sym);
                    self.chunk.code.push(OpCode::CaptureImplicit(sym));
                }

                let ret_type = return_type.and_then(|rt| self.convert_type(rt));
                let user_func = UserFunc {
                    params: param_syms,
                    uses: use_syms.clone(),
                    chunk: Rc::new(func_chunk),
                    is_static: *is_static,
                    is_generator,
                    statics: Rc::new(RefCell::new(HashMap::new())),
                    return_type: ret_type,
                    attributes: Vec::new(),
                    decl_line: self.line_at(span.start),
                    is_final: false,
                };
                let const_idx = self.add_constant(Val::Resource(Rc::new(user_func)));
                self.chunk
                    .code
                    .push(OpCode::Closure(const_idx as u32, use_syms.len() as u32));
            }
            Expr::Call { func, args, .. } => {
                match func {
                    Expr::Variable { span, .. } => {
                        let name = self.raw_text(*span);
                        if name.starts_with(b"$") {
                            self.emit_expr(func);
                        } else {
                            match self.resolve_function_or_constant(name, false) {
                                ResolvedName::Exact(name) => {
                                    let idx = self.add_constant(Val::String(name.into()));
                                    self.chunk.code.push(OpCode::Const(idx as u16));
                                }
                                ResolvedName::Fallback { namespaced, global } => {
                                    let idx = self.add_constant(Val::String(namespaced.into()));
                                    self.chunk.code.push(OpCode::Const(idx as u16));
                                    let global = self.add_constant(Val::String(global.into()));
                                    self.chunk.code.push(OpCode::NsFunctionFallback(global as u16));
                                }
                            }
                        }
                    }
                    _ => self.emit_expr(func),
                }

                self.emit_call_args(args);

                self.chunk.code.push(OpCode::Call(args.len() as u8));
            }
            Expr::Variable { span, .. } => {
                let name = self.get_text(*span);
                if name.starts_with(b"$") {
                    let var_name = &name[1..];
                    let sym = self.interner.intern(var_name);
                    self.chunk.code.push(OpCode::LoadVar(sym));
                } else {
                    // Constant fetch
                    match self.resolve_function_or_constant(self.raw_text(*span), true) {
                        ResolvedName::Exact(name) => {
                            let sym = self.interner.intern(&name);
                            self.chunk.code.push(OpCode::FetchGlobalConst(sym));
                        }
                        ResolvedName::Fallback { namespaced, global } => {
                            let namespaced = self.interner.intern(&namespaced);
                            let global = self.interner.intern(&global);
                            self.chunk.code.push(OpCode::FetchNsConst(namespaced, global));
                        }
                    }
                }
            }
            Expr::IndirectVariable { name, .. } => {
                self.emit_expr(name);
                self.chunk.code.push(OpCode::LoadVarDynamic);
            }
            Expr::Array { items, .. } => {
                self.chunk.code.push(OpCode::InitArray(items.len() as u32));
                for item in *items {
                    if item.unpack {
                        self.emit_expr(item.value);
                        self.chunk.code.push(OpCode::AddArrayUnpack);
                        continue;
                    }
                    if item.by_ref {
                        if let Some(key) = item.key {
                            self.emit_expr(key);
                        }
                        self.emit_ref_source(item.value);
                        self.chunk.code.push(OpCode::ArrayItemRef(item.key.is_some()));
                    } else if let Some(key) = item.key {
                        self.emit_expr(key);
                        self.emit_expr(item.value);
                        self.chunk.code.push(OpCode::AssignDim);
                    } else {
                        self.emit_expr(item.value);
                        self.chunk.code.push(OpCode::AppendArray);
                    }
                }
            }
            Expr::ArrayDimFetch { array, dim, .. } => {
                self.emit_chain_base(array);
                if let Some(d) = dim {
                    self.emit_expr(d);
                    self.chunk.code.push(OpCode::FetchDim);
                }
            }
            Expr::New { class, args, .. } => {
                if let Expr::AnonymousClass {
                    attributes,
                    modifiers,
                    extends,
                    implements,
                    members,
                    span,
                    ..
                } = class
                {
                    // Zend's naming: `<parent or first interface or "class">@anonymous`,
                    // then NUL, the declaring file, line and a per-file counter.
                    let base = extends
                        .as_ref()
                        .or(implements.first())
                        .map(|n| String::from_utf8_lossy(self.get_text(n.span)).into_owned())
                        .unwrap_or_else(|| "class".to_string());
                    let line = span.line_info(self.source).map_or(0, |info| info.line);
                    let anon_name = format!(
                        "{}@anonymous\0{}:{}${:x}",
                        base,
                        self.file_path.as_deref().unwrap_or(""),
                        line,
                        self.anonymous_class_counter
                    );
                    self.anonymous_class_counter += 1;
                    let class_sym = self.interner.intern(anon_name.as_bytes());

                    // Declared once, the first time the expression runs (ZEND_DECLARE_ANON_CLASS).
                    let skip = self.chunk.code.len();
                    self.chunk.code.push(OpCode::DeclareAnonClass(class_sym, 0));
                    self.emit_class_decl(class_sym, extends, implements, attributes, modifiers, members);
                    let end = self.chunk.code.len();
                    self.patch_jump(skip, end);

                    self.emit_call_args(args);
                    self.chunk
                        .code
                        .push(OpCode::New(class_sym, args.len() as u8));
                } else if let Expr::Variable { span, .. } = class {
                    let name = &self.resolve_class_name(self.raw_text(*span));
                    if !name.starts_with(b"$") {
                        let class_sym = self.interner.intern(name);

                        self.emit_call_args(args);

                        self.chunk
                            .code
                            .push(OpCode::New(class_sym, args.len() as u8));
                    } else {
                        // Dynamic new $var()
                        // Emit expression to get class name (string)
                        self.emit_expr(class);

                        self.emit_call_args(args);

                        self.chunk.code.push(OpCode::NewDynamic(args.len() as u8));
                    }
                } else {
                    // Complex expression for class name
                    self.emit_expr(class);

                    self.emit_call_args(args);

                    self.chunk.code.push(OpCode::NewDynamic(args.len() as u8));
                }
            }
            Expr::NullsafePropertyFetch {
                target, property, ..
            } => {
                self.emit_chain_base(target);
                self.emit_nullsafe_check();
                self.emit_property_fetch_tail(property);
            }
            Expr::PropertyFetch {
                target, property, ..
            } => {
                self.emit_chain_base(target);
                self.emit_property_fetch_tail(property);
            }
            Expr::NullsafeMethodCall {
                target,
                method,
                args,
                ..
            } => {
                self.emit_chain_base(target);
                self.emit_nullsafe_check();
                self.emit_method_call_tail(method, args);
            }
            Expr::MethodCall {
                target,
                method,
                args,
                ..
            } => {
                self.emit_chain_base(target);
                self.emit_method_call_tail(method, args);
            }
            Expr::StaticCall {
                class,
                method,
                args,
                ..
            } => {
                let mut class_emitted = false;
                if let Expr::Variable { span, .. } = class {
                    let class_name = &self.resolve_class_name(self.raw_text(*span));
                    if !class_name.starts_with(b"$") {
                        let class_sym = self.interner.intern(class_name);

                        if let Expr::Variable {
                            span: method_span, ..
                        } = method
                        {
                            let method_name = self.get_text(*method_span);
                            if !method_name.starts_with(b"$") {
                                self.emit_call_args(args);
                                let method_sym = self.interner.intern(method_name);
                                self.chunk.code.push(OpCode::CallStaticMethod(
                                    class_sym,
                                    method_sym,
                                    args.len() as u8,
                                ));
                                class_emitted = true;
                            }
                        }

                        if !class_emitted {
                            // Class is static, but method is dynamic: Class::$method()
                            let idx = self.add_constant(Val::String(class_name.to_vec().into()));
                            self.chunk.code.push(OpCode::Const(idx as u16));
                            self.emit_expr(method);
                            self.emit_call_args(args);
                            self.chunk
                                .code
                                .push(OpCode::CallStaticMethodDynamic(args.len() as u8));
                            class_emitted = true;
                        }
                    }
                }

                if !class_emitted {
                    // Dynamic static call: $class::$method()
                    self.emit_chain_base(class);
                    self.emit_expr(method);
                    self.emit_call_args(args);
                    self.chunk
                        .code
                        .push(OpCode::CallStaticMethodDynamic(args.len() as u8));
                }
            }
            Expr::ClassConstFetch {
                class, constant, ..
            } => {
                let mut is_class_keyword = false;
                if let Expr::Variable {
                    span: const_span, ..
                } = constant
                {
                    let const_name = self.get_text(*const_span);
                    if const_name.eq_ignore_ascii_case(b"class") {
                        is_class_keyword = true;
                    }
                }

                if let Expr::Variable { span, .. } = class {
                    let class_name = &self.resolve_class_name(self.raw_text(*span));
                    if !class_name.starts_with(b"$") {
                        if is_class_keyword {
                            let idx = self.add_constant(Val::String(class_name.to_vec().into()));
                            self.chunk.code.push(OpCode::Const(idx as u16));
                            return;
                        }

                        let class_sym = self.interner.intern(class_name);

                        if let Expr::Variable {
                            span: const_span, ..
                        } = constant
                        {
                            let const_name = self.get_text(*const_span);
                            if const_name.starts_with(b"$") {
                                let prop_name = &const_name[1..];
                                let prop_sym = self.interner.intern(prop_name);
                                self.chunk
                                    .code
                                    .push(OpCode::FetchStaticProp(class_sym, prop_sym));
                            } else {
                                let const_sym = self.interner.intern(const_name);
                                self.chunk
                                    .code
                                    .push(OpCode::FetchClassConst(class_sym, const_sym));
                            }
                        }
                        return;
                    }
                }

                // Dynamic class/object access
                self.emit_chain_base(class);
                if is_class_keyword {
                    self.chunk.code.push(OpCode::GetClass);
                } else {
                    if let Expr::Variable {
                        span: const_span, ..
                    } = constant
                    {
                        let const_name = self.get_text(*const_span);
                        if let Some(prop) = const_name.strip_prefix(b"$") {
                            // Dynamic class, static property: $obj::$prop
                            let idx = self.add_constant(Val::String(prop.to_vec().into()));
                            self.chunk.code.push(OpCode::GetClass);
                            self.chunk.code.push(OpCode::Const(idx as u16));
                            self.chunk.code.push(OpCode::FetchStaticPropR);
                        } else {
                            let const_sym = self.interner.intern(const_name);
                            self.chunk
                                .code
                                .push(OpCode::FetchClassConstDynamic(const_sym));
                        }
                    } else {
                        self.chunk.code.push(OpCode::Pop);
                        let idx = self.add_constant(Val::Null);
                        self.chunk.code.push(OpCode::Const(idx as u16));
                    }
                }
            }
            Expr::Assign { var, expr, .. } => match var {
                Expr::Variable { span, .. } => {
                    self.emit_expr(expr);
                    let name = self.get_text(*span);
                    if name.starts_with(b"$") {
                        let var_name = &name[1..];
                        let sym = self.interner.intern(var_name);
                        self.chunk.code.push(OpCode::StoreVar(sym));
                        self.chunk.code.push(OpCode::LoadVar(sym));
                    }
                }
                Expr::IndirectVariable { name, .. } => {
                    self.emit_expr(name);
                    self.emit_expr(expr);
                    self.chunk.code.push(OpCode::StoreVarDynamic);
                }
                Expr::PropertyFetch {
                    target, property, ..
                } => {
                    self.emit_write_base(target);
                    if let Some(sym) = self.static_prop_name(property) {
                        self.emit_expr(expr);
                        self.chunk.code.push(OpCode::AssignProp(sym));
                    } else {
                        // $obj->$name = v, $obj->{expr} = v
                        self.emit_expr(property);
                        self.emit_expr(expr);
                        self.chunk.code.push(OpCode::AssignPropDynamic);
                    }
                }
                Expr::ClassConstFetch {
                    class, constant, ..
                } => {
                    self.emit_expr(expr);
                    if let Expr::Variable { span, .. } = class {
                        let class_name = &self.resolve_class_name(self.raw_text(*span));
                        if !class_name.starts_with(b"$") {
                            let class_sym = self.interner.intern(class_name);

                            if let Expr::Variable {
                                span: const_span, ..
                            } = constant
                            {
                                let const_name = self.get_text(*const_span);
                                if const_name.starts_with(b"$") {
                                    let prop_name = &const_name[1..];
                                    let prop_sym = self.interner.intern(prop_name);
                                    self.chunk
                                        .code
                                        .push(OpCode::AssignStaticProp(class_sym, prop_sym));
                                }
                            }
                        }
                    }
                }
                Expr::ArrayDimFetch { .. } => self.emit_assign_dim(var, expr),
                Expr::Array { items, .. } => {
                    // list($a, $b, $c) = expr
                    // Emit the right-hand side expression (should be an array)
                    self.emit_expr(expr);

                    // Extract each element and assign to variables
                    for (i, item) in items.iter().enumerate() {
                        let value = item.value;
                        if let Expr::Variable { span, .. } = value {
                            let name = self.get_text(*span);
                            if name.starts_with(b"$") {
                                // Duplicate the array on stack for next iteration
                                self.chunk.code.push(OpCode::Dup);
                                // Push the index
                                let idx_val = Val::Int(i as i64);
                                let idx_const = self.add_constant(idx_val);
                                self.chunk.code.push(OpCode::Const(idx_const as u16));
                                // Fetch array[i] (pops index and duplicated array, pushes value, leaves original array)
                                self.chunk.code.push(OpCode::FetchDim);
                                // Store to variable (pops value)
                                let var_name = &name[1..];
                                let sym = self.interner.intern(var_name);
                                self.chunk.code.push(OpCode::StoreVar(sym));
                            }
                        }
                    }
                    // Leave the original array on the stack as the assignment result
                    // (statement-level Pop will remove it if needed)
                }
                _ => {}
            },
            Expr::AssignRef { var, expr, .. } => {
                match var {
                    Expr::Variable { span, .. } => {
                        // Check if expr is a variable
                        let mut handled = false;
                        if let Expr::Variable { span: src_span, .. } = expr {
                            let src_name = self.get_text(*src_span);
                            if src_name.starts_with(b"$") {
                                let src_sym = self.interner.intern(&src_name[1..]);
                                self.chunk.code.push(OpCode::MakeVarRef(src_sym));
                                handled = true;
                            }
                        }

                        if !handled {
                            self.emit_expr_for_write(expr);
                            self.chunk.code.push(OpCode::MakeRef);
                        }

                        let name = self.get_text(*span);
                        if name.starts_with(b"$") {
                            let var_name = &name[1..];
                            let sym = self.interner.intern(var_name);
                            self.chunk.code.push(OpCode::AssignRef(sym));
                            self.chunk.code.push(OpCode::LoadVar(sym));
                        }
                    }
                    Expr::ArrayDimFetch {
                        array: array_var,
                        dim,
                        ..
                    } => {
                        match array_var {
                            // Writing creates the variable without an "undefined" notice.
                            Expr::Variable { span, .. }
                                if self.get_text(*span).starts_with(b"$")
                                    && !matches!(self.get_text(*span), b"$this" | b"$GLOBALS") =>
                            {
                                let sym = self.interner.intern(&self.get_text(*span)[1..]);
                                self.chunk.code.push(OpCode::FetchW(sym));
                            }
                            _ => self.emit_expr(array_var),
                        }
                        if let Some(d) = dim {
                            self.emit_expr(d);
                        } else {
                            // TODO: Handle append
                            self.chunk.code.push(OpCode::Const(0));
                        }

                        let mut handled = false;
                        if let Expr::Variable { span: src_span, .. } = expr {
                            let src_name = self.get_text(*src_span);
                            if src_name.starts_with(b"$") {
                                let src_sym = self.interner.intern(&src_name[1..]);
                                self.chunk.code.push(OpCode::MakeVarRef(src_sym));
                                handled = true;
                            }
                        }

                        if !handled {
                            self.emit_expr_for_write(expr);
                            self.chunk.code.push(OpCode::MakeRef);
                        }

                        self.chunk.code.push(OpCode::AssignDimRef);

                        // Store back the updated array if target is a variable
                        if let Expr::Variable { span, .. } = array_var {
                            let name = self.get_text(*span);
                            if name.starts_with(b"$") {
                                let sym = self.interner.intern(&name[1..]);
                                self.chunk.code.push(OpCode::StoreVar(sym));
                            } else {
                                self.chunk.code.push(OpCode::Pop);
                            }
                        } else {
                            self.chunk.code.push(OpCode::Pop);
                        }
                    }
                    Expr::PropertyFetch {
                        target, property, ..
                    } => {
                        // $obj->p = &source
                        self.emit_write_base(target);
                        if let Some(sym) = self.static_prop_name(property) {
                            let name = self.interner.lookup(sym).unwrap_or(b"").to_vec();
                            let idx = self.add_constant(Val::String(name.into()));
                            self.chunk.code.push(OpCode::Const(idx as u16));
                        } else {
                            self.emit_expr(property);
                        }
                        self.emit_ref_source(expr);
                        self.chunk.code.push(OpCode::AssignObjRef);
                    }
                    Expr::ClassConstFetch {
                        class, constant, ..
                    } if self.static_member(class, constant).is_some() => {
                        // A::$p = &source
                        let (class_sym, prop_sym) = self.static_member(class, constant).unwrap();
                        for sym in [class_sym, prop_sym] {
                            let name = self.interner.lookup(sym).unwrap_or(b"").to_vec();
                            let idx = self.add_constant(Val::String(name.into()));
                            self.chunk.code.push(OpCode::Const(idx as u16));
                        }
                        self.emit_ref_source(expr);
                        self.chunk.code.push(OpCode::AssignStaticPropRef);
                    }
                    _ => {
                        // Unsupported target: evaluate the source so the expression
                        // still leaves one value.
                        self.emit_expr(expr);
                    }
                }
            }
            Expr::AssignOp { var, op, expr, .. } => {
                match var {
                    Expr::Variable { span, .. } => {
                        let name = self.get_text(*span);
                        if name.starts_with(b"$") {
                            let var_name = &name[1..];
                            let sym = self.interner.intern(var_name);

                            if let AssignOp::Coalesce = op {
                                // Check if set
                                self.chunk.code.push(OpCode::IssetVar(sym));
                                let jump_idx = self.chunk.code.len();
                                self.chunk.code.push(OpCode::JmpIfTrue(0));

                                // Not set: Evaluate expr, assign, load
                                self.emit_expr(expr);
                                self.chunk.code.push(OpCode::StoreVar(sym));
                                self.chunk.code.push(OpCode::LoadVar(sym));

                                let end_jump_idx = self.chunk.code.len();
                                self.chunk.code.push(OpCode::Jmp(0));

                                // Set: Load var
                                let label_set = self.chunk.code.len();
                                self.chunk.code[jump_idx] = OpCode::JmpIfTrue(label_set as u32);
                                self.chunk.code.push(OpCode::LoadVar(sym));

                                // End
                                let label_end = self.chunk.code.len();
                                self.chunk.code[end_jump_idx] = OpCode::Jmp(label_end as u32);
                                return;
                            }

                            // Load var
                            self.chunk.code.push(OpCode::LoadVar(sym));

                            // Evaluate expr
                            self.emit_expr(expr);

                            // Op
                            match op {
                                AssignOp::Plus => self.chunk.code.push(OpCode::Add),
                                AssignOp::Minus => self.chunk.code.push(OpCode::Sub),
                                AssignOp::Mul => self.chunk.code.push(OpCode::Mul),
                                AssignOp::Div => self.chunk.code.push(OpCode::Div),
                                AssignOp::Mod => self.chunk.code.push(OpCode::Mod),
                                AssignOp::Concat => self.chunk.code.push(OpCode::Concat),
                                AssignOp::Pow => self.chunk.code.push(OpCode::Pow),
                                AssignOp::BitAnd => self.chunk.code.push(OpCode::BitwiseAnd),
                                AssignOp::BitOr => self.chunk.code.push(OpCode::BitwiseOr),
                                AssignOp::BitXor => self.chunk.code.push(OpCode::BitwiseXor),
                                AssignOp::ShiftLeft => self.chunk.code.push(OpCode::ShiftLeft),
                                AssignOp::ShiftRight => self.chunk.code.push(OpCode::ShiftRight),
                                _ => {} // TODO: Implement other ops
                            }

                            // Store
                            self.chunk.code.push(OpCode::StoreVar(sym));
                            self.chunk.code.push(OpCode::LoadVar(sym));
                        }
                    }
                    Expr::IndirectVariable { name, .. } => {
                        self.emit_expr(name);
                        self.chunk.code.push(OpCode::Dup);

                        if let AssignOp::Coalesce = op {
                            self.chunk.code.push(OpCode::IssetVarDynamic);
                            let jump_idx = self.chunk.code.len();
                            self.chunk.code.push(OpCode::JmpIfTrue(0));

                            self.emit_expr(expr);
                            self.chunk.code.push(OpCode::StoreVarDynamic);

                            let end_jump_idx = self.chunk.code.len();
                            self.chunk.code.push(OpCode::Jmp(0));

                            let label_set = self.chunk.code.len();
                            self.chunk.code[jump_idx] = OpCode::JmpIfTrue(label_set as u32);
                            self.chunk.code.push(OpCode::LoadVarDynamic);

                            let label_end = self.chunk.code.len();
                            self.chunk.code[end_jump_idx] = OpCode::Jmp(label_end as u32);
                            return;
                        }

                        self.chunk.code.push(OpCode::LoadVarDynamic);
                        self.emit_expr(expr);

                        match op {
                            AssignOp::Plus => self.chunk.code.push(OpCode::Add),
                            AssignOp::Minus => self.chunk.code.push(OpCode::Sub),
                            AssignOp::Mul => self.chunk.code.push(OpCode::Mul),
                            AssignOp::Div => self.chunk.code.push(OpCode::Div),
                            AssignOp::Mod => self.chunk.code.push(OpCode::Mod),
                            AssignOp::Concat => self.chunk.code.push(OpCode::Concat),
                            AssignOp::Pow => self.chunk.code.push(OpCode::Pow),
                            AssignOp::BitAnd => self.chunk.code.push(OpCode::BitwiseAnd),
                            AssignOp::BitOr => self.chunk.code.push(OpCode::BitwiseOr),
                            AssignOp::BitXor => self.chunk.code.push(OpCode::BitwiseXor),
                            AssignOp::ShiftLeft => self.chunk.code.push(OpCode::ShiftLeft),
                            AssignOp::ShiftRight => self.chunk.code.push(OpCode::ShiftRight),
                            _ => {}
                        }

                        self.chunk.code.push(OpCode::StoreVarDynamic);
                    }
                    Expr::PropertyFetch {
                        target, property, ..
                    } => {
                        self.emit_write_base(target);

                        if self.static_prop_name(property).is_none() {
                            // $obj->$name op= expr
                            self.emit_expr(property);
                            self.chunk.code.push(OpCode::Dup2);
                            if let AssignOp::Coalesce = op {
                                // $obj->$name = $obj->$name ?? expr
                                self.chunk.code.push(OpCode::FetchPropDynamic);
                                let skip = self.chunk.code.len();
                                self.chunk.code.push(OpCode::Coalesce(0));
                                self.emit_expr(expr);
                                let assign = self.chunk.code.len();
                                self.patch_jump(skip, assign);
                                self.chunk.code.push(OpCode::AssignPropDynamic);
                            } else {
                                self.chunk.code.push(OpCode::FetchPropDynamic);
                                self.emit_expr(expr);
                                if let Some(code) = Self::assign_op_opcode(op) {
                                    self.chunk.code.push(code);
                                }
                                self.chunk.code.push(OpCode::AssignPropDynamic);
                            }
                            return;
                        }
                        self.chunk.code.push(OpCode::Dup);

                        if let Expr::Variable { span, .. } = property {
                            let name = self.get_text(*span);
                            if !name.starts_with(b"$") {
                                let sym = self.interner.intern(name);

                                if let AssignOp::Coalesce = op {
                                    self.chunk.code.push(OpCode::Dup);
                                    self.chunk.code.push(OpCode::IssetProp(sym));
                                    let jump_idx = self.chunk.code.len();
                                    self.chunk.code.push(OpCode::JmpIfTrue(0));

                                    self.emit_expr(expr);
                                    self.chunk.code.push(OpCode::AssignProp(sym));

                                    let end_jump_idx = self.chunk.code.len();
                                    self.chunk.code.push(OpCode::Jmp(0));

                                    let label_set = self.chunk.code.len();
                                    self.chunk.code[jump_idx] = OpCode::JmpIfTrue(label_set as u32);
                                    self.chunk.code.push(OpCode::FetchProp(sym));

                                    let label_end = self.chunk.code.len();
                                    self.chunk.code[end_jump_idx] = OpCode::Jmp(label_end as u32);
                                    return;
                                }

                                self.chunk.code.push(OpCode::FetchProp(sym));

                                self.emit_expr(expr);

                                match op {
                                    AssignOp::Plus => self.chunk.code.push(OpCode::Add),
                                    AssignOp::Minus => self.chunk.code.push(OpCode::Sub),
                                    AssignOp::Mul => self.chunk.code.push(OpCode::Mul),
                                    AssignOp::Div => self.chunk.code.push(OpCode::Div),
                                    AssignOp::Mod => self.chunk.code.push(OpCode::Mod),
                                    AssignOp::Concat => self.chunk.code.push(OpCode::Concat),
                                    AssignOp::Pow => self.chunk.code.push(OpCode::Pow),
                                    AssignOp::BitAnd => self.chunk.code.push(OpCode::BitwiseAnd),
                                    AssignOp::BitOr => self.chunk.code.push(OpCode::BitwiseOr),
                                    AssignOp::BitXor => self.chunk.code.push(OpCode::BitwiseXor),
                                    AssignOp::ShiftLeft => self.chunk.code.push(OpCode::ShiftLeft),
                                    AssignOp::ShiftRight => {
                                        self.chunk.code.push(OpCode::ShiftRight)
                                    }
                                    _ => {}
                                }

                                self.chunk.code.push(OpCode::AssignProp(sym));
                            }
                        }
                    }
                    Expr::ClassConstFetch {
                        class: Expr::Variable { span, .. },
                        constant,
                        ..
                    } => {
                        let class_name = &self.resolve_class_name(self.raw_text(*span));
                        if !class_name.starts_with(b"$") {
                            let class_sym = self.interner.intern(class_name);

                            if let Expr::Variable {
                                span: const_span, ..
                            } = constant
                            {
                                let const_name = self.get_text(*const_span);
                                if const_name.starts_with(b"$") {
                                    let prop_name = &const_name[1..];
                                    let prop_sym = self.interner.intern(prop_name);

                                    if let AssignOp::Coalesce = op {
                                        let idx = self.add_constant(Val::String(
                                            class_name.to_vec().into(),
                                        ));
                                        self.chunk.code.push(OpCode::Const(idx as u16));
                                        self.chunk.code.push(OpCode::IssetStaticProp(prop_sym));

                                        let jump_idx = self.chunk.code.len();
                                        self.chunk.code.push(OpCode::JmpIfFalse(0));

                                        self.chunk
                                            .code
                                            .push(OpCode::FetchStaticProp(class_sym, prop_sym));
                                        let jump_end_idx = self.chunk.code.len();
                                        self.chunk.code.push(OpCode::Jmp(0));

                                        let label_assign = self.chunk.code.len();
                                        self.chunk.code[jump_idx] =
                                            OpCode::JmpIfFalse(label_assign as u32);

                                        self.emit_expr(expr);
                                        self.chunk.code.push(OpCode::AssignStaticProp(
                                            class_sym, prop_sym,
                                        ));

                                        let label_end = self.chunk.code.len();
                                        self.chunk.code[jump_end_idx] =
                                            OpCode::Jmp(label_end as u32);
                                        return;
                                    }

                                    self.chunk
                                        .code
                                        .push(OpCode::FetchStaticProp(class_sym, prop_sym));
                                    self.emit_expr(expr);

                                    match op {
                                        AssignOp::Plus => self.chunk.code.push(OpCode::Add),
                                        AssignOp::Minus => self.chunk.code.push(OpCode::Sub),
                                        AssignOp::Mul => self.chunk.code.push(OpCode::Mul),
                                        AssignOp::Div => self.chunk.code.push(OpCode::Div),
                                        AssignOp::Mod => self.chunk.code.push(OpCode::Mod),
                                        AssignOp::Concat => {
                                            self.chunk.code.push(OpCode::Concat)
                                        }
                                        AssignOp::Pow => self.chunk.code.push(OpCode::Pow),
                                        AssignOp::BitAnd => {
                                            self.chunk.code.push(OpCode::BitwiseAnd)
                                        }
                                        AssignOp::BitOr => {
                                            self.chunk.code.push(OpCode::BitwiseOr)
                                        }
                                        AssignOp::BitXor => {
                                            self.chunk.code.push(OpCode::BitwiseXor)
                                        }
                                        AssignOp::ShiftLeft => {
                                            self.chunk.code.push(OpCode::ShiftLeft)
                                        }
                                        AssignOp::ShiftRight => {
                                            self.chunk.code.push(OpCode::ShiftRight)
                                        }
                                        _ => {}
                                    }

                                    self.chunk
                                        .code
                                        .push(OpCode::AssignStaticProp(class_sym, prop_sym));
                                }
                            }
                        }
                    }
                    // $o->p[k] op= v, A::$p[k] op= v, ...: the element's own
                    // slot, changed in place (a plain $var base keeps the path
                    // below).
                    Expr::ArrayDimFetch { .. }
                        if !matches!(op, AssignOp::Coalesce)
                            && !matches!(Self::flatten_dim_fetch(var).0, Expr::Variable { .. })
                            && Self::assign_op_opcode(op).is_some() =>
                    {
                        let Some(bin) = Self::assign_op_opcode(op) else { unreachable!() };
                        self.emit_expr_for_update(var);
                        self.chunk.code.push(OpCode::Dup);
                        self.emit_expr(expr);
                        self.chunk.code.push(bin);
                        self.chunk.code.push(OpCode::StoreToSlot);
                    }
                    Expr::ArrayDimFetch { .. } => {
                        let (base, keys) = Self::flatten_dim_fetch(var);

                        // 1. Emit base array
                        self.emit_expr(base);

                        // 2. Emit keys
                        for key in &keys {
                            if let Some(k) = key {
                                self.emit_expr(k);
                            } else {
                                // Append not supported in AssignOp (e.g. $a[] += 1 is invalid)
                                // But maybe $a[] ??= 1 is valid? No, ??= is assign op.
                                // PHP Fatal error:  Cannot use [] for reading
                                // So we can assume keys are present for AssignOp (read-modify-write)
                                // But wait, $a[] = 1 is valid. $a[] += 1 is NOT valid.
                                // So we can panic or emit error if key is None.
                                // For now, push 0 or null?
                                // Actually, let's just push 0 as placeholder, but it will fail at runtime if used for reading.
                                self.chunk.code.push(OpCode::Const(0));
                            }
                        }

                        // 3. Fetch value (peek array & keys, push val)
                        // Stack: [array, keys...]
                        self.chunk
                            .code
                            .push(OpCode::FetchNestedDim(keys.len() as u8));
                        // Stack: [array, keys..., val]

                        if let AssignOp::Coalesce = op {
                            let jump_idx = self.chunk.code.len();
                            self.chunk.code.push(OpCode::Coalesce(0));

                            // If null, evaluate rhs
                            self.emit_expr(expr);

                            let label_store = self.chunk.code.len();
                            self.chunk.code[jump_idx] = OpCode::Coalesce(label_store as u32);
                        } else {
                            // 4. Emit expr (rhs)
                            self.emit_expr(expr);
                            // Stack: [array, keys..., val, rhs]

                            // 5. Op
                            match op {
                                AssignOp::Plus => self.chunk.code.push(OpCode::Add),
                                AssignOp::Minus => self.chunk.code.push(OpCode::Sub),
                                AssignOp::Mul => self.chunk.code.push(OpCode::Mul),
                                AssignOp::Div => self.chunk.code.push(OpCode::Div),
                                AssignOp::Mod => self.chunk.code.push(OpCode::Mod),
                                AssignOp::Concat => self.chunk.code.push(OpCode::Concat),
                                AssignOp::Pow => self.chunk.code.push(OpCode::Pow),
                                AssignOp::BitAnd => self.chunk.code.push(OpCode::BitwiseAnd),
                                AssignOp::BitOr => self.chunk.code.push(OpCode::BitwiseOr),
                                AssignOp::BitXor => self.chunk.code.push(OpCode::BitwiseXor),
                                AssignOp::ShiftLeft => self.chunk.code.push(OpCode::ShiftLeft),
                                AssignOp::ShiftRight => self.chunk.code.push(OpCode::ShiftRight),
                                _ => {}
                            }
                        }

                        // 6. Store result back
                        // Stack: [array, keys..., result]
                        self.chunk
                            .code
                            .push(OpCode::StoreNestedDim(keys.len() as u8));
                        // Stack: [new_array] (StoreNestedDim pushes the modified array back? No, wait.)

                        // Wait, I checked StoreNestedDim implementation.
                        // It does NOT push anything back.
                        // But assign_nested_dim pushes new_handle back!
                        // And StoreNestedDim calls assign_nested_dim.
                        // So StoreNestedDim DOES push new_array back.

                        // So Stack: [new_array]

                        // 7. Update variable if base was a variable
                        if let Expr::Variable { span, .. } = base {
                            let name = self.get_text(*span);
                            if name.starts_with(b"$") {
                                let var_name = &name[1..];
                                let sym = self.interner.intern(var_name);
                                self.chunk.code.push(OpCode::StoreVar(sym));
                                self.chunk.code.push(OpCode::LoadVar(sym));
                            }
                        }
                    }
                    _ => {} // TODO: Other targets
                }
            }
            Expr::MagicConst { kind, span, .. } => {
                // Handle magic constants like __DIR__, __FILE__, etc.
                let value = match kind {
                    MagicConstKind::Dir => {
                        // __DIR__ returns the directory of the current file
                        if let Some(ref path) = self.file_path {
                            if let Some(parent) = Path::new(path).parent() {
                                let dir_str = parent.to_string_lossy();
                                Val::String(Rc::new(dir_str.as_bytes().to_vec()))
                            } else {
                                Val::String(Rc::new(b".".to_vec()))
                            }
                        } else {
                            // No file path tracked, return current directory
                            Val::String(Rc::new(b".".to_vec()))
                        }
                    }
                    MagicConstKind::File => {
                        // __FILE__ returns the full path of the current file
                        if let Some(ref path) = self.file_path {
                            Val::String(Rc::new(path.as_bytes().to_vec()))
                        } else {
                            // No file path tracked, return empty string
                            Val::String(Rc::new(Vec::new()))
                        }
                    }
                    MagicConstKind::Line => {
                        // __LINE__ returns the current line number
                        let line = self.get_line_number(span.start);
                        Val::Int(line)
                    }
                    MagicConstKind::Class => {
                        // __CLASS__ returns the class name (or empty if not in a class)
                        if let Some(class_sym) = self.current_class {
                            if let Some(class_name) = self.interner.lookup(class_sym) {
                                Val::String(Rc::new(class_name.to_vec()))
                            } else {
                                Val::String(Rc::new(Vec::new()))
                            }
                        } else {
                            Val::String(Rc::new(Vec::new()))
                        }
                    }
                    MagicConstKind::Trait => {
                        // __TRAIT__ returns the trait name
                        if let Some(trait_sym) = self.current_trait {
                            if let Some(trait_name) = self.interner.lookup(trait_sym) {
                                Val::String(Rc::new(trait_name.to_vec()))
                            } else {
                                Val::String(Rc::new(Vec::new()))
                            }
                        } else {
                            Val::String(Rc::new(Vec::new()))
                        }
                    }
                    MagicConstKind::Method => {
                        // __METHOD__ returns Class::method or just method
                        if let Some(func_sym) = self.current_function {
                            if let Some(func_name) = self.interner.lookup(func_sym) {
                                Val::String(Rc::new(func_name.to_vec()))
                            } else {
                                Val::String(Rc::new(Vec::new()))
                            }
                        } else {
                            Val::String(Rc::new(Vec::new()))
                        }
                    }
                    MagicConstKind::Function => {
                        // __FUNCTION__ returns the function name (without class)
                        if let Some(func_sym) = self.current_function {
                            if let Some(func_name) = self.interner.lookup(func_sym) {
                                // Strip class prefix if present (Class::method -> method)
                                let name_str = func_name;
                                if let Some(pos) = name_str.iter().position(|&b| b == b':') {
                                    if pos + 1 < name_str.len() && name_str[pos + 1] == b':' {
                                        // Found ::, return part after it
                                        Val::String(Rc::new(name_str[pos + 2..].to_vec()))
                                    } else {
                                        Val::String(Rc::new(name_str.to_vec()))
                                    }
                                } else {
                                    Val::String(Rc::new(name_str.to_vec()))
                                }
                            } else {
                                Val::String(Rc::new(Vec::new()))
                            }
                        } else {
                            Val::String(Rc::new(Vec::new()))
                        }
                    }
                    MagicConstKind::Namespace => {
                        // __NAMESPACE__ returns the namespace name
                        if let Some(ns_sym) = self.current_namespace {
                            if let Some(ns_name) = self.interner.lookup(ns_sym) {
                                Val::String(Rc::new(ns_name.to_vec()))
                            } else {
                                Val::String(Rc::new(Vec::new()))
                            }
                        } else {
                            Val::String(Rc::new(Vec::new()))
                        }
                    }
                    MagicConstKind::Property => {
                        // __PROPERTY__ (PHP 8.3+) - not commonly used yet
                        // Would need property context tracking
                        Val::String(Rc::new(Vec::new()))
                    }
                };
                let idx = self.add_constant(value);
                self.chunk.code.push(OpCode::Const(idx as u16));
            }
            _ => {}
        }
    }

    /// A call argument. `$a[k]` keeps the element's own slot (not a copy), so
    /// a by-reference parameter binds to it; the compiler cannot see whether
    /// the parameter is by-reference (Zend's FETCH_DIM_FUNC_ARG).
    fn emit_arg(&mut self, value: &Expr) {
        if let Expr::ArrayDimFetch {
            array,
            dim: Some(dim),
            span,
        } = value
        {
            self.at_line(span.start, |this| {
                this.emit_expr(array);
                this.emit_expr(dim);
                this.chunk.code.push(OpCode::FetchDimArg);
            });
        } else if let Expr::Variable { span, .. } = value
            && self.get_text(*span).starts_with(b"$")
            && self.get_text(*span) != b"$this"
        {
            let sym = self.interner.intern(&self.get_text(*span)[1..]);
            self.at_line(span.start, |this| this.chunk.code.push(OpCode::LoadVarArg(sym)));
        } else {
            // `f($a[])` may pass by reference: whether `[]` reads is only
            // known when the call runs.
            self.append_fetch_ok = true;
            self.emit_expr(value);
            self.append_fetch_ok = false;
        }
    }

    /// The `$name`s written in a span of source, in order of first use. An
    /// arrow function captures these; a name that only appears inside a
    /// string is captured too, which is harmless (it is never read).
    fn variables_in(&self, span: crate::parser::span::Span) -> Vec<Vec<u8>> {
        let text = &self.source[span.start..span.end.min(self.source.len())];
        let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c >= 0x80;
        let mut out: Vec<Vec<u8>> = Vec::new();
        let mut i = 0;
        while i < text.len() {
            if text[i] == b'$' && i + 1 < text.len() && (ident(text[i + 1]) && !text[i + 1].is_ascii_digit()) {
                let start = i + 1;
                let mut end = start;
                while end < text.len() && ident(text[end]) {
                    end += 1;
                }
                let name = text[start..end].to_vec();
                if !out.contains(&name) {
                    out.push(name);
                }
                i = end;
            } else {
                i += 1;
            }
        }
        out
    }

    /// A parameter default as PHP prints it in a function prototype.
    /// Reference: $PHP_SRC_PATH/Zend/zend_inheritance.c - zend_get_function_declaration
    fn default_display(&self, expr: &Expr) -> String {
        match expr {
            Expr::Variable { span, .. } if !self.get_text(*span).starts_with(b"$") => {
                String::from_utf8_lossy(self.get_text(*span)).into_owned()
            }
            Expr::ClassConstFetch { class, constant, .. } => match (class, constant) {
                (Expr::Variable { span: c, .. }, Expr::Variable { span: k, .. })
                    if !self.get_text(*k).starts_with(b"$") =>
                {
                    format!(
                        "{}::{}",
                        String::from_utf8_lossy(self.get_text(*c)),
                        String::from_utf8_lossy(self.get_text(*k))
                    )
                }
                _ => "<expression>".into(),
            },
            Expr::Integer { .. }
            | Expr::Float { .. }
            | Expr::String { .. }
            | Expr::Boolean { .. }
            | Expr::Null { .. }
            | Expr::Array { .. }
            | Expr::Unary { .. } => match self.eval_constant_expr(expr) {
                Val::Bool(true) => "true".into(),
                Val::Bool(false) => "false".into(),
                Val::Null => "null".into(),
                Val::Int(i) => i.to_string(),
                Val::Float(f) => crate::core::value::php_float_to_string(
                    f,
                    crate::core::value::FloatPrecision::echo(),
                    'E',
                ),
                Val::String(s) => {
                    let shown = String::from_utf8_lossy(&s[..s.len().min(10)]).into_owned();
                    format!("'{}{}'", shown, if s.len() > 10 { "..." } else { "" })
                }
                Val::Array(a) if a.map.is_empty() => "[]".into(),
                Val::Array(_) | Val::ConstArray(_) => "[...]".into(),
                _ => "<expression>".into(),
            },
            _ => "<expression>".into(),
        }
    }

    /// Push an expression's value the way isset(), empty() and `??` read
    /// their operand (Zend's BP_VAR_IS fetches): an undefined variable, a
    /// missing key or property, or a lookup on null is quietly null.
    fn emit_expr_quiet(&mut self, expr: &Expr) {
        self.short_circuit(expr, |this| this.emit_expr_quiet_inner(expr));
    }

    fn emit_expr_quiet_inner(&mut self, expr: &Expr) {
        match expr {
            Expr::Variable { span, .. }
                if self.get_text(*span).starts_with(b"$") && self.get_text(*span) != b"$this" =>
            {
                let sym = self.interner.intern(&self.get_text(*span)[1..]);
                self.chunk.code.push(OpCode::FetchIs(sym));
            }
            Expr::ArrayDimFetch {
                array,
                dim: Some(dim),
                span,
            } => {
                self.emit_expr_quiet_inner(array);
                self.emit_expr(dim);
                self.at_line(span.start, |this| this.chunk.code.push(OpCode::FetchDimIs));
            }
            Expr::PropertyFetch {
                target, property, span, ..
            }
            | Expr::NullsafePropertyFetch {
                target, property, span, ..
            } => {
                self.emit_expr_quiet_inner(target);
                if matches!(expr, Expr::NullsafePropertyFetch { .. }) {
                    self.emit_nullsafe_check();
                }
                match property {
                    Expr::Variable { span: name_span, .. }
                        if !self.get_text(*name_span).starts_with(b"$") =>
                    {
                        let name = self.get_text(*name_span).to_vec();
                        let idx = self.add_constant(Val::String(name.into()));
                        self.chunk.code.push(OpCode::Const(idx as u16));
                    }
                    other => self.emit_expr(other),
                }
                self.at_line(span.start, |this| this.chunk.code.push(OpCode::FetchObjIs));
            }
            _ => self.emit_chain_base(expr),
        }
    }

    /// A call's arguments. With a spread (`...$a`) or named (`x: 1`)
    /// argument the count is only known at run time: BeginArgs .. EndArgs
    /// hand the real count and the names to the call opcode that follows.
    fn emit_call_args(&mut self, args: &[Arg]) {
        if !args.iter().any(|a| a.unpack || a.name.is_some()) {
            for arg in args {
                self.emit_arg(arg.value);
            }
            return;
        }
        self.chunk.code.push(OpCode::BeginArgs);
        for arg in args {
            if arg.unpack {
                self.emit_expr(arg.value);
                self.chunk.code.push(OpCode::SpreadArg);
            } else if let Some(name) = arg.name {
                let sym = self.interner.intern(self.get_text(name.span));
                self.emit_arg(arg.value);
                self.chunk.code.push(OpCode::NamedArg(sym));
            } else {
                self.emit_arg(arg.value);
                self.chunk.code.push(OpCode::PositionalArg);
            }
        }
        self.chunk.code.push(OpCode::EndArgs);
    }

    /// Push the right-hand side of `= &expr` as a reference.
    fn emit_ref_source(&mut self, expr: &Expr) {
        if let Expr::Variable { span, .. } = expr
            && self.get_text(*span).starts_with(b"$")
            && self.get_text(*span) != b"$this"
        {
            let sym = self.interner.intern(&self.get_text(*span)[1..]);
            self.chunk.code.push(OpCode::MakeVarRef(sym));
        } else {
            self.emit_expr_for_write(expr);
            self.chunk.code.push(OpCode::MakeRef);
        }
    }

    /// The slot ++/-- change in place: like emit_expr_for_write, but the last
    /// step is a read-write fetch, which gives the element a slot of its own.
    fn emit_expr_for_update(&mut self, expr: &Expr) {
        match expr {
            Expr::ArrayDimFetch { array, dim: Some(dim), .. } => {
                self.emit_expr_for_write(array);
                self.emit_expr(dim);
                self.chunk.code.push(OpCode::FetchDimRw);
            }
            _ => self.emit_expr_for_write(expr),
        }
    }

    /// Push the slot an lvalue lives in (not a copy of its value), creating it
    /// when missing, as Zend's FETCH_*_W opcodes do: `&$a['k']`, `&$o->p`,
    /// `&A::$p`. Anything else is evaluated normally.
    fn emit_expr_for_write(&mut self, expr: &Expr) {
        match expr {
            Expr::Variable { span, .. } if self.get_text(*span).starts_with(b"$") => {
                let name = &self.get_text(*span)[1..];
                if name == b"this" || name == b"GLOBALS" {
                    self.emit_expr(expr);
                } else {
                    let sym = self.interner.intern(name);
                    self.chunk.code.push(OpCode::FetchW(sym));
                }
            }
            Expr::ArrayDimFetch { array, dim, .. } => {
                self.emit_expr_for_write(array);
                match dim {
                    Some(d) => self.emit_expr(d),
                    None => {
                        let idx = self.add_constant(Val::AppendPlaceholder);
                        self.chunk.code.push(OpCode::Const(idx as u16));
                    }
                }
                self.chunk.code.push(OpCode::FetchDimW);
            }
            Expr::PropertyFetch {
                target, property, ..
            } => {
                self.emit_write_base(target);
                if let Some(sym) = self.static_prop_name(property) {
                    let name = self.interner.lookup(sym).unwrap_or(b"").to_vec();
                    let idx = self.add_constant(Val::String(name.into()));
                    self.chunk.code.push(OpCode::Const(idx as u16));
                } else {
                    self.emit_expr(property);
                }
                self.chunk.code.push(OpCode::FetchObjW);
            }
            Expr::ClassConstFetch {
                class, constant, ..
            } if self.static_member(class, constant).is_some() => {
                let (class_sym, prop_sym) = self.static_member(class, constant).unwrap();
                for sym in [class_sym, prop_sym] {
                    let name = self.interner.lookup(sym).unwrap_or(b"").to_vec();
                    let idx = self.add_constant(Val::String(name.into()));
                    self.chunk.code.push(OpCode::Const(idx as u16));
                }
                self.chunk.code.push(OpCode::FetchStaticPropW);
            }
            _ => self.emit_expr(expr),
        }
    }

    /// The binary opcode behind a compound assignment (`+=` -> Add); None for `??=`.
    fn assign_op_opcode(op: &AssignOp) -> Option<OpCode> {
        Some(match op {
            AssignOp::Plus => OpCode::Add,
            AssignOp::Minus => OpCode::Sub,
            AssignOp::Mul => OpCode::Mul,
            AssignOp::Div => OpCode::Div,
            AssignOp::Mod => OpCode::Mod,
            AssignOp::Concat => OpCode::Concat,
            AssignOp::Pow => OpCode::Pow,
            AssignOp::BitAnd => OpCode::BitwiseAnd,
            AssignOp::BitOr => OpCode::BitwiseOr,
            AssignOp::BitXor => OpCode::BitwiseXor,
            AssignOp::ShiftLeft => OpCode::ShiftLeft,
            AssignOp::ShiftRight => OpCode::ShiftRight,
            _ => return None,
        })
    }

    /// The name of `$obj->name` when it is a literal identifier.
    fn static_prop_name(&mut self, property: &Expr) -> Option<Symbol> {
        if let Expr::Variable { span, .. } = property {
            let name = self.get_text(*span);
            if !name.starts_with(b"$") {
                return Some(self.interner.intern(name));
            }
        }
        None
    }

    /// `Class::$prop` with a literal class name, as (class, prop).
    fn static_member(&mut self, class: &Expr, constant: &Expr) -> Option<(Symbol, Symbol)> {
        let (Expr::Variable { span: class_span, .. }, Expr::Variable { span: prop_span, .. }) =
            (class, constant)
        else {
            return None;
        };
        let class_name = &self.resolve_class_name(self.raw_text(*class_span));
        let prop_name = self.get_text(*prop_span);
        if class_name.starts_with(b"$") || !prop_name.starts_with(b"$") {
            return None;
        }
        Some((self.interner.intern(class_name), self.interner.intern(&prop_name[1..])))
    }

    /// `$base[k1]..[kN] = expr`: read the base, update a copy, store it back
    /// into the base, and leave the assigned value as the result.
    fn emit_assign_dim(&mut self, var: &Expr, expr: &Expr) {
        enum StoreBack {
            Var(Symbol),
            Prop(Symbol),
            PropDynamic,
            StaticProp(Symbol, Symbol),
            Discard,
        }
        let (base, keys) = Self::flatten_dim_fetch(var);

        // Push the operands the store-back needs, then the current array.
        let (store, extras) = match base {
            Expr::Variable { span, .. } if self.get_text(*span).starts_with(b"$") => {
                let name = &self.get_text(*span)[1..];
                let sym = self.interner.intern(name);
                if name == b"GLOBALS" || name == b"this" {
                    self.emit_expr(base);
                } else {
                    // Writing creates the variable without an "undefined" notice.
                    self.chunk.code.push(OpCode::FetchW(sym));
                }
                (StoreBack::Var(sym), 0)
            }
            Expr::PropertyFetch {
                target, property, ..
            } => {
                // The current value is read as a write fetch (FETCH_OBJ_W): a
                // missing property is created, not reported as undefined.
                self.emit_write_base(target);
                if let Some(sym) = self.static_prop_name(property) {
                    self.chunk.code.push(OpCode::Dup);
                    let name = self.interner.lookup(sym).unwrap_or(b"").to_vec();
                    let idx = self.add_constant(Val::String(name.into()));
                    self.chunk.code.push(OpCode::Const(idx as u16));
                    self.chunk.code.push(OpCode::FetchObjIs);
                    (StoreBack::Prop(sym), 1)
                } else {
                    self.emit_expr(property);
                    self.chunk.code.push(OpCode::Dup2);
                    self.chunk.code.push(OpCode::FetchObjIs);
                    (StoreBack::PropDynamic, 2)
                }
            }
            Expr::ClassConstFetch {
                class, constant, ..
            } if self.static_member(class, constant).is_some() => {
                let (class_sym, prop_sym) = self.static_member(class, constant).unwrap();
                self.emit_expr(base);
                (StoreBack::StaticProp(class_sym, prop_sym), 0)
            }
            _ => {
                self.emit_expr(base);
                (StoreBack::Discard, 0)
            }
        };

        for key in &keys {
            if let Some(k) = key {
                self.emit_expr(k);
            } else {
                let idx = self.add_constant(Val::AppendPlaceholder);
                self.chunk.code.push(OpCode::Const(idx as u16));
            }
        }
        self.emit_expr(expr);
        self.chunk
            .code
            .push(OpCode::AssignNestedDim(keys.len() as u8, extras));

        // Stack: [value, extras.., new array]; store back, leaving [value].
        match store {
            StoreBack::Var(sym) => self.chunk.code.push(OpCode::StoreVar(sym)),
            StoreBack::Prop(sym) => {
                self.chunk.code.push(OpCode::AssignProp(sym));
                self.chunk.code.push(OpCode::Pop);
            }
            StoreBack::PropDynamic => {
                self.chunk.code.push(OpCode::AssignPropDynamic);
                self.chunk.code.push(OpCode::Pop);
            }
            StoreBack::StaticProp(class_sym, prop_sym) => {
                self.chunk
                    .code
                    .push(OpCode::AssignStaticProp(class_sym, prop_sym));
                self.chunk.code.push(OpCode::Pop);
            }
            StoreBack::Discard => self.chunk.code.push(OpCode::Pop),
        }
    }

    fn flatten_dim_fetch<'a, 'ast>(
        mut expr: &'a Expr<'ast>,
    ) -> (&'a Expr<'ast>, Vec<Option<&'a Expr<'ast>>>) {
        let mut keys = Vec::new();
        while let Expr::ArrayDimFetch { array, dim, .. } = expr {
            keys.push(*dim);
            expr = array;
        }
        keys.reverse();
        (expr, keys)
    }

    fn add_constant(&mut self, val: Val) -> usize {
        self.chunk.constants.push(val);
        self.chunk.constants.len() - 1
    }

    /// A constant expression's value (zend_compile_const_expr): literals,
    /// and arrays of them, are kept as values; anything else (constants,
    /// operators, `self::X`, `new`) becomes a thunk that the VM evaluates
    /// when the value is needed (zval_update_constant_ex).
    fn const_initializer(&mut self, expr: &Expr) -> Val {
        if Self::is_literal(expr) {
            return self.eval_constant_expr(expr);
        }
        let (line, line_starts) = self.child_lines(expr.span().start);
        let compile_error = self.compile_error.clone();
        let mut thunk = Emitter::new(self.source, self.interner);
        thunk.compile_error = compile_error;
        thunk.compile_warnings = self.compile_warnings.clone();
        thunk.imports = self.imports.clone();
        thunk.current_line = line;
        thunk.line_starts = line_starts;
        thunk.file_path = self.file_path.clone();
        thunk.current_class = self.current_class;
        thunk.current_trait = self.current_trait;
        thunk.current_namespace = self.current_namespace;
        thunk.current_function = Some(thunk.interner.intern(b"{constant}"));
        thunk.chunk.strict_types = self.chunk.strict_types;
        let (chunk, _) = thunk.compile_arrow_body(expr);
        Val::Resource(Rc::new(ConstExpr(Rc::new(chunk))))
    }

    /// A literal, a signed number literal, or an array of literals.
    fn is_literal(expr: &Expr) -> bool {
        match expr {
            Expr::Integer { .. }
            | Expr::Float { .. }
            | Expr::String { .. }
            | Expr::Boolean { .. }
            | Expr::Null { .. } => true,
            Expr::Unary {
                op: UnaryOp::Minus | UnaryOp::Plus,
                expr: Expr::Integer { .. } | Expr::Float { .. },
                ..
            } => true,
            Expr::Array { items, .. } => items.iter().all(|item| {
                !item.unpack
                    && !item.by_ref
                    && item.key.is_none_or(Self::is_literal)
                    && Self::is_literal(item.value)
            }),
            _ => false,
        }
    }

    fn eval_constant_expr(&self, expr: &Expr) -> Val {
        match expr {
            Expr::Unary {
                op: op @ (UnaryOp::Minus | UnaryOp::Plus),
                expr: inner,
                ..
            } => match (op, self.eval_constant_expr(inner)) {
                (UnaryOp::Minus, Val::Int(i)) => match i.checked_neg() {
                    Some(n) => Val::Int(n),
                    None => Val::Float(-(i as f64)),
                },
                (UnaryOp::Minus, Val::Float(f)) => Val::Float(-f),
                (_, v) => v,
            },
            Expr::Integer { value, .. } => integer_literal(value),
            Expr::Float { value, .. } => Val::Float(float_literal(value)),
            Expr::String { value, .. } => Val::String(string_literal(value).into()),
            Expr::Boolean { value, .. } => Val::Bool(*value),
            Expr::Null { .. } => Val::Null,
            Expr::Array { items, .. } => {
                if items.is_empty() {
                    Val::Array(Rc::new(crate::core::value::ArrayData::new()))
                } else {
                    // Build a compile-time constant array template
                    use crate::core::value::ConstArrayKey;
                    use indexmap::IndexMap;

                    let mut const_array = IndexMap::new();
                    let mut next_index = 0i64;

                    for item in *items {
                        if item.unpack {
                            // Array unpacking not supported in constant expressions
                            continue;
                        }

                        let val = self.eval_constant_expr(item.value);

                        if let Some(key_expr) = item.key {
                            let key_val = self.eval_constant_expr(key_expr);
                            let key = match key_val {
                                Val::Int(i) => {
                                    if i >= next_index {
                                        next_index = i + 1;
                                    }
                                    ConstArrayKey::Int(i)
                                }
                                Val::String(s) => ConstArrayKey::Str(s),
                                Val::Float(f) => {
                                    let i = f as i64;
                                    if i >= next_index {
                                        next_index = i + 1;
                                    }
                                    ConstArrayKey::Int(i)
                                }
                                Val::Bool(b) => {
                                    let i = if b { 1 } else { 0 };
                                    if i >= next_index {
                                        next_index = i + 1;
                                    }
                                    ConstArrayKey::Int(i)
                                }
                                _ => ConstArrayKey::Int(next_index),
                            };
                            const_array.insert(key, val);
                        } else {
                            const_array.insert(ConstArrayKey::Int(next_index), val);
                            next_index += 1;
                        }
                    }

                    Val::ConstArray(Rc::new(const_array))
                }
            }
            Expr::ClassConstFetch { class, constant, .. } => {
                let class_name = match class {
                    Expr::Variable { name, .. } => Some(self.get_text(*name)),
                    _ => None,
                };
                let const_name = match constant {
                    Expr::Variable { name, .. } => Some(self.get_text(*name)),
                    _ => None,
                };

                if let (Some(class_name), Some(const_name)) = (class_name, const_name) {
                    if class_name == b"Attribute" || class_name.ends_with(b"\\Attribute") {
                        let value = match const_name {
                            b"TARGET_CLASS" => Some(1),
                            b"TARGET_METHOD" => Some(2),
                            b"TARGET_FUNCTION" => Some(4),
                            b"TARGET_PROPERTY" => Some(8),
                            b"TARGET_CLASS_CONSTANT" => Some(16),
                            b"TARGET_PARAMETER" => Some(32),
                            b"TARGET_ALL" => Some(63),
                            b"IS_REPEATABLE" => Some(64),
                            _ => None,
                        };
                        if let Some(v) = value {
                            return Val::Int(v);
                        }
                    }
                }

                Val::Null
            }
            Expr::Binary { left, op, right, .. } => {
                match op {
                    BinaryOp::BitOr => {
                        let l = self.eval_constant_expr(left);
                        let r = self.eval_constant_expr(right);
                        if let (Val::Int(li), Val::Int(ri)) = (l, r) {
                            Val::Int(li | ri)
                        } else {
                            Val::Null
                        }
                    }
                    _ => Val::Null,
                }
            }
            _ => Val::Null,
        }
    }

    /// The source text of a span. A fully qualified name (`\Foo\Bar`) loses its
    /// leading backslash: it names the same global symbol, and every lookup
    /// keys on the unqualified form.
    fn get_text(&self, span: crate::parser::span::Span) -> &'src [u8] {
        let text = &self.source[span.start..span.end];
        match text {
            [b'\\', next, ..] if next.is_ascii_alphabetic() || *next == b'_' || *next >= 0x80 => &text[1..],
            _ => text,
        }
    }

    /// Emit constants for static property access (Class::$property)
    /// Returns true if successfully emitted, false if not a valid static property reference
    fn emit_static_property_access(&mut self, class: &Expr, constant: &Expr) -> bool {
        if let (
            Expr::Variable {
                name: class_span, ..
            },
            Expr::Variable {
                name: prop_span, ..
            },
        ) = (class, constant)
        {
            let class_name = &self.resolve_class_name(self.raw_text(*class_span));
            let prop_name = self.get_text(*prop_span);

            // Valid static property: Class::$property (class name without $, property with $)
            if !class_name.starts_with(b"$") && prop_name.starts_with(b"$") {
                let class_idx = self.add_constant(Val::String(Rc::new(class_name.to_vec())));
                let prop_idx = self.add_constant(Val::String(Rc::new(prop_name[1..].to_vec())));
                self.chunk.code.push(OpCode::Const(class_idx as u16));
                self.chunk.code.push(OpCode::Const(prop_idx as u16));
                return true;
            }
        }
        false
    }

    /// Calculate line number from byte offset (1-indexed)
    fn get_line_number(&self, offset: usize) -> i64 {
        let mut line = 1i64;
        for (i, &byte) in self.source.iter().enumerate() {
            if i >= offset {
                break;
            }
            if byte == b'\n' {
                line += 1;
            }
        }
        line
    }

    /// Convert AST Type to ReturnType
    fn convert_type(&mut self, ty: &Type) -> Option<ReturnType> {
        match ty {
            Type::Simple(tok) => match tok.kind {
                TokenKind::TypeInt => Some(ReturnType::Int),
                TokenKind::TypeFloat => Some(ReturnType::Float),
                TokenKind::TypeString => Some(ReturnType::String),
                TokenKind::TypeBool => Some(ReturnType::Bool),
                TokenKind::Array => Some(ReturnType::Array),
                TokenKind::TypeObject => Some(ReturnType::Object),
                TokenKind::TypeVoid => Some(ReturnType::Void),
                TokenKind::TypeNever => Some(ReturnType::Never),
                TokenKind::TypeMixed => Some(ReturnType::Mixed),
                TokenKind::TypeNull => Some(ReturnType::Null),
                TokenKind::TypeTrue => Some(ReturnType::True),
                TokenKind::TypeFalse => Some(ReturnType::False),
                TokenKind::TypeCallable => Some(ReturnType::Callable),
                TokenKind::TypeIterable => Some(ReturnType::Iterable),
                TokenKind::Static => Some(ReturnType::Static),
                _ => None,
            },
            Type::Name(name) => {
                let sym = self.class_sym(name.span);
                Some(ReturnType::Named(sym))
            }
            Type::Union(types) => {
                let converted: Vec<_> = types.iter().filter_map(|t| self.convert_type(t)).collect();
                if converted.is_empty() {
                    None
                } else {
                    Some(ReturnType::Union(converted))
                }
            }
            Type::Intersection(types) => {
                let converted: Vec<_> = types.iter().filter_map(|t| self.convert_type(t)).collect();
                if converted.is_empty() {
                    None
                } else {
                    Some(ReturnType::Intersection(converted))
                }
            }
            Type::Nullable(inner) => self
                .convert_type(inner)
                .map(|t| ReturnType::Nullable(Box::new(t))),
        }
    }
}

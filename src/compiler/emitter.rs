use crate::compiler::chunk::{CatchEntry, CodeChunk, FuncParam, ReturnType, UserFunc};
use crate::core::interner::Interner;
use crate::core::value::{Symbol, Val, Visibility};
use crate::parser::ast::{
    Arg, AssignOp, AttributeGroup, BinaryOp, CastKind, ClassMember, Expr, IncludeKind, MagicConstKind,
    Stmt, StmtId, Type, UnaryOp,
};
use crate::parser::lexer::token::{Token, TokenKind};
use crate::runtime::context::{AttributeArg, AttributeInstance};
use crate::vm::opcode::OpCode;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

/// Unescape a double-quoted string (or heredoc / interpolated part) the way
/// Zend's scanner does: \n \t \r \v \e \f \\ \$ \", octal \[0-7]{1,3},
/// hex \x[0-9a-f]{1,2} and \u{codepoint}; any other backslash is kept.
/// Reference: $PHP_SRC_PATH/Zend/zend_language_scanner.l - zend_scan_escape_string
fn unescape_string(s: &[u8]) -> Vec<u8> {
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
fn integer_literal(value: &[u8]) -> Val {
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
fn float_literal(value: &[u8]) -> f64 {
    let text: String = value.iter().filter(|b| **b != b'_').map(|b| char::from(*b)).collect();
    text.parse().unwrap_or(0.0)
}

/// The value of a string literal token: '...' (only \\ and \' are escapes),
/// "..." (full escapes) or an unquoted interpolation/heredoc part.
fn string_literal(value: &[u8]) -> Vec<u8> {
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
    /// Source line of the statement or expression being emitted; every opcode
    /// is tagged with it (CodeChunk::lines), for errors, exceptions and traces.
    current_line: u32,
    /// Byte offsets where each source line starts, shared with child emitters.
    line_starts: Option<Rc<Vec<usize>>>,
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
            current_line: 1,
            line_starts: None,
        }
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

        for stmt in stmts {
            self.emit_stmt(stmt);
        }
        self.finish()
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
                    let mut method_emitter = Emitter::new(self.source, self.interner);
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
                        if p_name.starts_with(b"$") {
                            let sym = method_emitter.interner.intern(&p_name[1..]);
                            let param_type = info.ty.and_then(|ty| method_emitter.convert_type(ty));
                            let default_value = info
                                .default
                                .map(|expr| method_emitter.eval_constant_expr(expr));

                            param_syms.push(FuncParam {
                                name: sym,
                                by_ref: info.by_ref,
                                param_type,
                                is_variadic: info.variadic,
                                default_value,
                                default_display: info.default.map(|e| method_emitter.default_display(e).into()),
                                attributes: method_emitter.build_attribute_instances(info.attributes),
                            });

                            if let Some(default_expr) = info.default {
                                let val = method_emitter.eval_constant_expr(default_expr);
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
                            let val = self.eval_constant_expr(default_expr);
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
                        Some(d) => self.eval_constant_expr(d),
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

                        let val = self
                            .get_literal_value(entry.value)
                            .unwrap_or_else(|| Val::Null);
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
                        let trait_str = self.get_text(trait_name.span);
                        let trait_sym = self.interner.intern(trait_str);
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
            .map(|parent| self.interner.intern(self.get_text(parent.span)));
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
            let interface_sym = self.interner.intern(self.get_text(interface.span));
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
                let attr_name = self.get_text(attr.name.span);
                // #[\Foo] names the global class Foo
                let attr_name = attr_name.strip_prefix(b"\\").unwrap_or(attr_name);
                let attr_sym = self.interner.intern(attr_name);
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
                let attr_name = self.get_text(attr.name.span);
                // #[\Foo] names the global class Foo
                let attr_name = attr_name.strip_prefix(b"\\").unwrap_or(attr_name);
                let attr_sym = self.interner.intern(attr_name);
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
                let attr_name = self.get_text(attr.name.span);
                // #[\Foo] names the global class Foo
                let attr_name = attr_name.strip_prefix(b"\\").unwrap_or(attr_name);
                let attr_sym = self.interner.intern(attr_name);
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
        match stmt {
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
                    let name_str = self.get_text(c.name.span);
                    let sym = self.interner.intern(name_str);

                    // Value must be constant expression.
                    // For now, we only support literals or simple expressions we can evaluate at compile time?
                    // Or we can emit code to evaluate it and then define it?
                    // PHP `const` requires constant expression.
                    // If we emit code, we can use `DefGlobalConst` which takes a value index?
                    // No, `DefGlobalConst` takes `val_idx` which implies it's in the constant table.
                    // So we must evaluate it at compile time.

                    let val = self.get_literal_value(c.value).unwrap_or_else(|| Val::Null);

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
                                    self.emit_expr(target); // [obj]
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
                            self.emit_expr(target);
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

                            if is_static_prop {
                                if let Expr::Variable { span, .. } = class {
                                    let name = self.get_text(*span);
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
                let func_name_str = self.get_text(name.span);
                let func_sym = self.interner.intern(func_name_str);

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
                    let mut func_emitter = Emitter::new(self.source, self.interner);
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
                    if p_name.starts_with(b"$") {
                        let sym = func_emitter.interner.intern(&p_name[1..]);
                        let param_type = info.ty.and_then(|ty| func_emitter.convert_type(ty));
                        let default_value = info
                            .default
                            .map(|expr| func_emitter.eval_constant_expr(expr));

                        param_syms.push(FuncParam {
                            name: sym,
                            by_ref: info.by_ref,
                            param_type,
                            is_variadic: info.variadic,
                            default_value,
                            default_display: info.default.map(|e| func_emitter.default_display(e).into()),
                            attributes: func_emitter.build_attribute_instances(info.attributes),
                        });

                        if let Some(default_expr) = info.default {
                            let val = func_emitter.eval_constant_expr(default_expr);
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
                let class_name_str = self.get_text(name.span);
                let class_sym = self.interner.intern(class_name_str);
                self.emit_class_decl(class_sym, extends, implements, attributes, modifiers, members);
            }
            Stmt::Interface {
                name,
                members,
                extends,
                ..
            } => {
                let name_str = self.get_text(name.span);
                let sym = self.interner.intern(name_str);
                let interface_line = self.current_line;

                self.chunk.code.push(OpCode::DefInterface(sym));

                for interface in *extends {
                    let interface_str = self.get_text(interface.span);
                    let interface_sym = self.interner.intern(interface_str);
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
                let name_str = self.get_text(name.span);
                let sym = self.interner.intern(name_str);

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
                let enum_name_str = self.get_text(name.span);
                let enum_sym = self.interner.intern(enum_name_str);

                let backed_flag = match backed_type {
                    Some(Type::Simple(tok)) if tok.kind == TokenKind::TypeInt => 1,
                    Some(Type::Simple(tok)) if tok.kind == TokenKind::TypeString => 2,
                    _ => 0,
                };

                self.chunk.code.push(OpCode::DefEnum(enum_sym, backed_flag));

                for interface in *implements {
                    let interface_str = self.get_text(interface.span);
                    let interface_sym = self.interner.intern(interface_str);
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

                // Pop switch cond
                self.chunk.code.push(OpCode::Pop);

                if let Some(def_lbl) = default_label {
                    self.chunk.code.push(OpCode::Jmp(def_lbl as u32));
                } else {
                    // No default, jump to end
                    self.chunk.code.push(OpCode::Jmp(jump_over_dispatch as u32));
                    // Will be patched to end_label
                }

                let end_label = self.chunk.code.len();
                self.patch_jump(jump_over_dispatch, end_label);

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
                        let type_name = self.get_text(ty.span);
                        let type_sym = self.interner.intern(type_name);

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
        self.at_line(expr.span().start, |this| this.emit_expr_inner(expr));
    }

    fn emit_expr_inner(&mut self, expr: &Expr) {
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
                                    return;
                                } else {
                                    // Bare class name - push as string constant
                                    Val::String(name.to_vec().into())
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
                                self.emit_expr(target);
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
                                self.emit_expr(target);
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
                        self.emit_expr(target);
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
                        self.emit_expr(target);
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
                                        let name = self.get_text(*span);
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
                                let name = self.get_text(*span);
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
                    let mut func_emitter = Emitter::new(self.source, self.interner);
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
                    if p_name.starts_with(b"$") {
                        let sym = func_emitter.interner.intern(&p_name[1..]);
                        let param_type = info.ty.and_then(|ty| func_emitter.convert_type(ty));
                        let default_value = info
                            .default
                            .map(|expr| func_emitter.eval_constant_expr(expr));

                        param_syms.push(FuncParam {
                            name: sym,
                            by_ref: info.by_ref,
                            param_type,
                            is_variadic: info.variadic,
                            default_value,
                            default_display: info.default.map(|e| func_emitter.default_display(e).into()),
                            attributes: func_emitter.build_attribute_instances(info.attributes),
                        });

                        if let Some(default_expr) = info.default {
                            let val = func_emitter.eval_constant_expr(default_expr);
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
                let mut func_emitter = Emitter::new(self.source, self.interner);
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
                    let Some(p_name) = p_name.strip_prefix(b"$") else {
                        continue;
                    };
                    let sym = func_emitter.interner.intern(p_name);
                    let param_type = param.ty.and_then(|ty| func_emitter.convert_type(ty));
                    let default_value = param.default.map(|e| func_emitter.eval_constant_expr(e));
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
                        let name = self.get_text(*span);
                        if name.starts_with(b"$") {
                            self.emit_expr(func);
                        } else {
                            let idx = self.add_constant(Val::String(name.to_vec().into()));
                            self.chunk.code.push(OpCode::Const(idx as u16));
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
                    let sym = self.interner.intern(name);
                    self.chunk.code.push(OpCode::FetchGlobalConst(sym));
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
                self.emit_expr(array);
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
                    let name = self.get_text(*span);
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
            Expr::PropertyFetch {
                target, property, ..
            } => {
                self.emit_expr(target);
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
                    eprintln!("Property is not Variable: {:?}", property);
                    // Handle dynamic property fetch with expression: $this->{$expr}
                    self.emit_expr(property);
                    self.chunk.code.push(OpCode::FetchPropDynamic);
                }
            }
            Expr::MethodCall {
                target,
                method,
                args,
                ..
            } => {
                self.emit_expr(target);
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
            Expr::StaticCall {
                class,
                method,
                args,
                ..
            } => {
                let mut class_emitted = false;
                if let Expr::Variable { span, .. } = class {
                    let class_name = self.get_text(*span);
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
                    self.emit_expr(class);
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
                    let class_name = self.get_text(*span);
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
                self.emit_expr(class);
                if is_class_keyword {
                    self.chunk.code.push(OpCode::GetClass);
                } else {
                    if let Expr::Variable {
                        span: const_span, ..
                    } = constant
                    {
                        let const_name = self.get_text(*const_span);
                        if const_name.starts_with(b"$") {
                            // TODO: Dynamic class, static property: $obj::$prop
                            self.chunk.code.push(OpCode::Pop);
                            let idx = self.add_constant(Val::Null);
                            self.chunk.code.push(OpCode::Const(idx as u16));
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
                    self.emit_expr(target);
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
                        let class_name = self.get_text(*span);
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
                        self.emit_expr(target);
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
                        self.emit_expr(target);

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
                        class, constant, ..
                    } => {
                        if let Expr::Variable { span, .. } = class {
                            let class_name = self.get_text(*span);
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
            self.emit_expr(value);
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
                    crate::core::value::FloatPrecision::ECHO,
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
                self.emit_expr_quiet(array);
                self.emit_expr(dim);
                self.at_line(span.start, |this| this.chunk.code.push(OpCode::FetchDimIs));
            }
            Expr::PropertyFetch {
                target, property, span, ..
            } => {
                self.emit_expr_quiet(target);
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
            _ => self.emit_expr(expr),
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
                self.emit_expr(target);
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
        let class_name = self.get_text(*class_span);
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
                self.emit_expr(target);
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

    fn eval_constant_expr(&self, expr: &Expr) -> Val {
        match expr {
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
            let class_name = self.get_text(*class_span);
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
                let name_str = self.get_text(name.span);
                let sym = self.interner.intern(name_str);
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

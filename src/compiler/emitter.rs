use crate::compiler::chunk::{CatchEntry, CodeChunk, FuncParam, ReturnType, UserFunc};
use crate::core::interner::Interner;
use crate::core::value::{Symbol, Val, Visibility};
use crate::parser::ast::{
    AssignOp, AttributeGroup, BinaryOp, CastKind, ClassKind, ClassMember, Expr, ExprId,
    IncludeKind, JsxAttribute, JsxChild, MagicConstKind, Name, ObjectKey, Stmt, StmtId, Type,
    UnaryOp, UseItem, UseKind,
};
use crate::parser::lexer::token::{Token, TokenKind};
use crate::parser::span::Span;
use crate::runtime::context::{AttributeArg, AttributeInstance};
use crate::vm::opcode::OpCode;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

/// Unescape a double-quoted string, processing escape sequences like \n, \r, \t, etc.
fn unescape_string(s: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'\\' && i + 1 < s.len() {
            match s[i + 1] {
                b'n' => result.push(b'\n'),
                b'r' => result.push(b'\r'),
                b't' => result.push(b'\t'),
                b'\\' => result.push(b'\\'),
                b'$' => result.push(b'$'),
                b'"' => result.push(b'"'),
                b'\'' => result.push(b'\''),
                b'v' => result.push(b'\x0B'), // vertical tab
                b'e' => result.push(b'\x1B'), // escape
                b'f' => result.push(b'\x0C'), // form feed
                b'0' => result.push(b'\0'),   // null byte
                // Hexadecimal: \xHH
                b'x' if i + 3 < s.len() => {
                    if let (Some(h1), Some(h2)) = (
                        char::from(s[i + 2]).to_digit(16),
                        char::from(s[i + 3]).to_digit(16),
                    ) {
                        result.push((h1 * 16 + h2) as u8);
                        i += 2; // Skip the two hex digits
                    } else {
                        result.push(b'\\');
                        result.push(s[i + 1]);
                    }
                }
                // Octal: \nnn (up to 3 digits)
                b'0'..=b'7' => {
                    let mut octal_val = s[i + 1] - b'0';
                    let mut consumed = 1;
                    if i + 2 < s.len() && (b'0'..=b'7').contains(&s[i + 2]) {
                        octal_val = octal_val * 8 + (s[i + 2] - b'0');
                        consumed = 2;
                        if i + 3 < s.len() && (b'0'..=b'7').contains(&s[i + 3]) {
                            octal_val = octal_val * 8 + (s[i + 3] - b'0');
                            consumed = 3;
                        }
                    }
                    result.push(octal_val);
                    i += consumed;
                }
                _ => {
                    // Unknown escape, keep both characters
                    result.push(b'\\');
                    result.push(s[i + 1]);
                }
            }
            i += 2;
        } else {
            result.push(s[i]);
            i += 1;
        }
    }
    result
}

struct LoopInfo {
    break_jumps: Vec<usize>,
    continue_jumps: Vec<usize>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NameKind {
    Class,
    Function,
    Const,
}

#[derive(Clone, Default)]
struct UseAliases {
    classes: HashMap<Vec<u8>, Vec<u8>>,
    functions: HashMap<Vec<u8>, Vec<u8>>,
    consts: HashMap<Vec<u8>, Vec<u8>>,
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
    use_aliases: UseAliases,
    // For eval(): inherit strict_types from parent scope if not explicitly declared
    inherited_strict_types: Option<bool>,
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
            use_aliases: UseAliases::default(),
            inherited_strict_types: None,
        }
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
        let mut declared_props = HashSet::new();
        for member in members {
            if let ClassMember::Property { entries, .. } = member {
                for entry in *entries {
                    let prop_name_str = self.get_text(entry.name.span);
                    let prop_name = if prop_name_str.starts_with(b"$") {
                        &prop_name_str[1..]
                    } else {
                        prop_name_str
                    };
                    declared_props.insert(self.interner.intern(prop_name));
                }
            }
        }

        for member in members {
            match member {
                ClassMember::Method {
                    name,
                    body,
                    params,
                    modifiers,
                    return_type,
                    attributes,
                    ..
                } => {
                    let method_name_str = self.get_text(name.span);
                    let method_sym = self.interner.intern(method_name_str);
                    let visibility = self.get_visibility(modifiers);
                    let is_static = modifiers.iter().any(|t| t.kind == TokenKind::Static);
                    let is_abstract = modifiers.iter().any(|t| t.kind == TokenKind::Abstract);
                    let is_ctor = method_name_str.eq_ignore_ascii_case(b"__construct");

                    let mut promoted_params: Vec<(
                        Symbol,
                        Symbol,
                        Option<&Type>,
                        bool,
                        Visibility,
                    )> =
                        Vec::new();
                    if is_ctor {
                        for param in *params {
                            let is_promoted = param.modifiers.iter().any(|t| {
                                matches!(
                                    t.kind,
                                    TokenKind::Public | TokenKind::Protected | TokenKind::Private
                                )
                            });
                            if !is_promoted {
                                continue;
                            }

                            let param_name = self.get_text(param.name.span);
                            let param_sym = if param_name.starts_with(b"$") {
                                self.interner.intern(&param_name[1..])
                            } else {
                                self.interner.intern(param_name)
                            };
                            let prop_sym = param_sym;
                            let prop_visibility = self.get_visibility(param.modifiers);
                            let is_readonly =
                                param.modifiers.iter().any(|t| t.kind == TokenKind::Readonly);

                            promoted_params
                                .push((prop_sym, param_sym, param.ty, is_readonly, prop_visibility));
                        }
                    }

                    if is_ctor {
                        for (prop_sym, _param_sym, ty, is_readonly, prop_visibility) in
                            &promoted_params
                        {
                            if !declared_props.contains(prop_sym) {
                                let type_hint_opt = ty.and_then(|t| self.convert_type(t));
                                let type_hint_idx = if let Some(ref th) = type_hint_opt {
                                    self.add_constant(Val::Resource(Rc::new(th.clone())))
                                } else {
                                    self.add_constant(Val::Null)
                                };
                                let default_idx = if type_hint_opt.is_some() || *is_readonly {
                                    self.add_constant(Val::Uninitialized)
                                } else {
                                    self.add_constant(Val::Null)
                                };

                                self.chunk.code.push(OpCode::DefProp(
                                    class_sym,
                                    *prop_sym,
                                    default_idx as u16,
                                    *prop_visibility,
                                    type_hint_idx as u32,
                                    *is_readonly,
                                ));
                                declared_props.insert(*prop_sym);
                            }
                        }
                    }

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
                    let mut method_emitter = Emitter::new(self.source, self.interner);
                    method_emitter.file_path = self.file_path.clone();
                    method_emitter.current_class = Some(class_sym);
                    method_emitter.current_namespace = self.current_namespace;
                    method_emitter.use_aliases = self.use_aliases.clone();
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
                                attributes: method_emitter
                                    .build_attribute_instances(info.attributes),
                            });

                        if info.variadic {
                            method_emitter
                                .chunk
                                .code
                                .push(OpCode::RecvVariadic(i as u32));
                        } else if let Some(default_expr) = info.default {
                            let val = method_emitter.eval_constant_expr(default_expr);
                            let idx = method_emitter.add_constant(val);
                            method_emitter
                                .chunk
                                .code
                                .push(OpCode::RecvInit(i as u32, idx as u16));
                        } else {
                            method_emitter.chunk.code.push(OpCode::Recv(i as u32));
                        }
                        }
                    }

                    if is_ctor {
                        for (prop_sym, param_sym, _ty, _is_readonly, _prop_visibility) in
                            &promoted_params
                        {
                            method_emitter.chunk.code.push(OpCode::FetchThis);
                            method_emitter.chunk.code.push(OpCode::LoadVar(*param_sym));
                            method_emitter.chunk.code.push(OpCode::AssignProp(*prop_sym));
                            method_emitter.chunk.code.push(OpCode::Pop);
                        }
                    }

                    let (method_chunk, is_generator) = method_emitter.compile(body);

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
                        self.chunk
                            .code
                            .push(OpCode::AddMethodAttribute(class_sym, method_sym, attr_idx));
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
                                class_sym, prop_sym, attr_idx, is_static,
                            ));
                        }
                    }
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
                                class_sym, const_sym, attr_idx,
                            ));
                        }
                    }
                }
                ClassMember::Case {
                    name,
                    value,
                    payload,
                    ..
                } => {
                    let case_name_str = self.get_text(name.span);
                    let case_sym = self.interner.intern(case_name_str);
                    let (val_idx, has_value) = if let Some(expr) = value {
                        let val = self.eval_constant_expr(expr);
                        (self.add_constant(val) as u16, true)
                    } else {
                        (0u16, false)
                    };

                    self.chunk
                        .code
                        .push(OpCode::DefEnumCase(class_sym, case_sym, val_idx, has_value));

                    if let Some(payload) = payload {
                        if !payload.is_empty() {
                            use crate::core::value::ConstArrayKey;
                            use indexmap::IndexMap;

                            let mut const_array = IndexMap::new();
                            for (idx, param) in payload.iter().enumerate() {
                                let mut name = self.get_text(param.name.span);
                                if name.starts_with(b"$") {
                                    name = &name[1..];
                                }
                                let val = Val::String(name.to_vec().into());
                                const_array.insert(ConstArrayKey::Int(idx as i64), val);
                            }
                            let payload_idx =
                                self.add_constant(Val::ConstArray(Rc::new(const_array)));
                            self.chunk.code.push(OpCode::DefEnumCasePayload(
                                class_sym,
                                case_sym,
                                payload_idx as u16,
                            ));
                        }
                    }
                }
                ClassMember::Embed { types, .. } => {
                    for type_name in *types {
                        let type_str = self.get_text(type_name.span);
                        let resolved = self.resolve_ref_name_bytes(type_str, NameKind::Class);
                        let embed_sym = self.interner.intern(&resolved);
                        let type_hint_idx =
                            self.add_constant(Val::Resource(Rc::new(ReturnType::Named(embed_sym))));
                        let default_idx = self.add_constant(Val::Uninitialized);

                        self.chunk.code.push(OpCode::DefProp(
                            class_sym,
                            embed_sym,
                            default_idx as u16,
                            Visibility::Public,
                            type_hint_idx as u32,
                            false,
                        ));
                        self.chunk
                            .code
                            .push(OpCode::EmbedStruct(class_sym, embed_sym));
                    }
                }
                ClassMember::TraitUse { traits, .. } => {
                    for trait_name in *traits {
                        let trait_str = self.get_text(trait_name.span);
                        let resolved = self.resolve_ref_name_bytes(trait_str, NameKind::Class);
                        let trait_sym = self.interner.intern(&resolved);
                        self.chunk.code.push(OpCode::UseTrait(class_sym, trait_sym));
                    }
                }
                _ => {}
            }
        }
    }

    fn emit_class_attributes(&mut self, class_sym: Symbol, attributes: &[AttributeGroup]) {
        for attr_group in attributes {
            for attr in attr_group.attributes {
                let attr_name = self.get_text(attr.name.span);
                let resolved = self.resolve_ref_name_bytes(attr_name, NameKind::Class);
                let attr_sym = self.interner.intern(&resolved);
                let mut args = Vec::new();

                for arg in attr.args {
                    let val = self.eval_constant_expr(arg.value);
                    let name = arg
                        .name
                        .map(|token| self.interner.intern(self.get_text(token.span)));
                    args.push(AttributeArg { name, value: val });
                }

                let instance = AttributeInstance {
                    name: attr_sym,
                    args,
                };
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
                let resolved = self.resolve_ref_name_bytes(attr_name, NameKind::Class);
                let attr_sym = self.interner.intern(&resolved);
                let mut args = Vec::new();

                for arg in attr.args {
                    let val = self.eval_constant_expr(arg.value);
                    let name = arg
                        .name
                        .map(|token| self.interner.intern(self.get_text(token.span)));
                    args.push(AttributeArg { name, value: val });
                }

                let instance = AttributeInstance {
                    name: attr_sym,
                    args,
                };
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
                let resolved = self.resolve_ref_name_bytes(attr_name, NameKind::Class);
                let attr_sym = self.interner.intern(&resolved);
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
        match stmt {
            Stmt::Declare { declares, body, .. } => {
                // PHP: declare(strict_types=1) is per-file and affects calls made from this file.
                // The parser already validates strict_types is an integer literal 0/1.
                for item in *declares {
                    let key = self.get_text(item.key.span);
                    if key.eq_ignore_ascii_case(b"strict_types") {
                        if let Expr::Integer { value, .. } = item.value {
                            // Integers may contain '_' separators.
                            let mut num: u64 = 0;
                            for b in *value {
                                if *b == b'_' {
                                    continue;
                                }
                                if !b.is_ascii_digit() {
                                    num = 0;
                                    break;
                                }
                                num = num.saturating_mul(10).saturating_add((b - b'0') as u64);
                            }
                            self.chunk.strict_types = num == 1;
                        }
                    }
                }

                for s in *body {
                    self.emit_stmt(s);
                }
            }
            Stmt::Namespace { name, body, .. } => {
                let new_ns = name.map(|n| {
                    let raw = self.get_text(n.span);
                    let resolved = self.normalize_namespace_decl(raw);
                    self.interner.intern(&resolved)
                });

                if let Some(stmts) = body {
                    let prev_ns = self.current_namespace;
                    let prev_aliases = self.use_aliases.clone();
                    self.current_namespace = new_ns;
                    self.reset_use_aliases();
                    for s in *stmts {
                        self.emit_stmt(s);
                    }
                    self.current_namespace = prev_ns;
                    self.use_aliases = prev_aliases;
                } else {
                    self.current_namespace = new_ns;
                    self.reset_use_aliases();
                }
            }
            Stmt::Use { uses, .. } => {
                for item in *uses {
                    self.register_use_item(item);
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
            Stmt::TypeAlias { .. } => {}
            Stmt::Const { consts, .. } => {
                for c in *consts {
                    let name_str = self.get_text(c.name.span);
                    let resolved = self.resolve_decl_name_bytes(name_str);
                    let sym = self.interner.intern(&resolved);

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

                    let val = if let Some(expr) = default_expr {
                        self.eval_constant_expr(expr)
                    } else {
                        Val::Null
                    };

                    let idx = self.add_constant(val);
                    self.chunk.code.push(OpCode::BindStatic(name, idx as u16));
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
                let resolved_name = self.resolve_decl_name_bytes(func_name_str);
                let func_sym = self.interner.intern(&resolved_name);

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
                let mut func_emitter = Emitter::new(self.source, self.interner);
                func_emitter.file_path = self.file_path.clone();
                func_emitter.current_function = Some(func_sym);
                func_emitter.current_namespace = self.current_namespace;
                func_emitter.use_aliases = self.use_aliases.clone();
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
                            attributes: func_emitter.build_attribute_instances(info.attributes),
                        });

                        if info.variadic {
                            func_emitter
                                .chunk
                                .code
                                .push(OpCode::RecvVariadic(i as u32));
                        } else if let Some(default_expr) = info.default {
                            let val = func_emitter.eval_constant_expr(default_expr);
                            let idx = func_emitter.add_constant(val);
                            func_emitter
                                .chunk
                                .code
                                .push(OpCode::RecvInit(i as u32, idx as u16));
                        } else {
                            func_emitter.chunk.code.push(OpCode::Recv(i as u32));
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
                };

                let func_res = Val::Resource(Rc::new(user_func));
                let const_idx = self.add_constant(func_res);

                self.chunk
                    .code
                    .push(OpCode::DefFunc(func_sym, const_idx as u32));
            }
            Stmt::Class {
                kind,
                name,
                members,
                extends,
                implements,
                attributes,
                modifiers,
                ..
            } => {
                let class_name_str = self.get_text(name.span);
                let resolved_class = self.resolve_decl_name_bytes(class_name_str);
                let class_sym = self.interner.intern(&resolved_class);

                let parent_sym = if let Some(parent_name) = extends {
                    let parent_str = self.get_text(parent_name.span);
                    let resolved_parent = self.resolve_ref_name_bytes(parent_str, NameKind::Class);
                    Some(self.interner.intern(&resolved_parent))
                } else {
                    None
                };

                self.chunk
                    .code
                    .push(OpCode::DefClass(class_sym, parent_sym));

                if *kind == ClassKind::Struct {
                    self.chunk.code.push(OpCode::MarkStruct(class_sym));
                }

                // Check if class is abstract
                let is_abstract = modifiers.iter().any(|m| m.kind == TokenKind::Abstract);
                if is_abstract {
                    self.chunk.code.push(OpCode::MarkAbstract(class_sym));
                }

                for interface in *implements {
                    let interface_str = self.get_text(interface.span);
                    let resolved_iface =
                        self.resolve_ref_name_bytes(interface_str, NameKind::Class);
                    let interface_sym = self.interner.intern(&resolved_iface);
                    self.chunk
                        .code
                        .push(OpCode::AddInterface(class_sym, interface_sym));
                }

                // Check for #[AllowDynamicProperties] attribute
                for attr_group in *attributes {
                    for attr in attr_group.attributes {
                        let attr_name = self.get_text(attr.name.span);
                        let resolved =
                            self.resolve_ref_name_bytes(attr_name, NameKind::Class);
                        // Check for both fully qualified and simple name
                        if resolved.eq_ignore_ascii_case(b"AllowDynamicProperties")
                            || resolved.ends_with(b"\\AllowDynamicProperties")
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

                // Finalize class: validate interfaces, abstract methods, etc.
                self.chunk.code.push(OpCode::FinalizeClass(class_sym));
            }
            Stmt::Interface {
                name,
                members,
                extends,
                ..
            } => {
                let name_str = self.get_text(name.span);
                let resolved_name = self.resolve_decl_name_bytes(name_str);
                let sym = self.interner.intern(&resolved_name);

                self.chunk.code.push(OpCode::DefInterface(sym));

                for interface in *extends {
                    let interface_str = self.get_text(interface.span);
                    let resolved_iface =
                        self.resolve_ref_name_bytes(interface_str, NameKind::Class);
                    let interface_sym = self.interner.intern(&resolved_iface);
                    self.chunk
                        .code
                        .push(OpCode::AddInterface(sym, interface_sym));
                }

                let prev_class = self.current_class;
                self.current_class = Some(sym);
                self.emit_members(sym, members);
                self.current_class = prev_class;
            }
            Stmt::Trait { name, members, .. } => {
                let name_str = self.get_text(name.span);
                let resolved_name = self.resolve_decl_name_bytes(name_str);
                let sym = self.interner.intern(&resolved_name);

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
                let resolved_name = self.resolve_decl_name_bytes(enum_name_str);
                let enum_sym = self.interner.intern(&resolved_name);

                let backed_flag = match backed_type {
                    Some(Type::Simple(tok)) if tok.kind == TokenKind::TypeInt => 1,
                    Some(Type::Simple(tok)) if tok.kind == TokenKind::TypeString => 2,
                    _ => 0,
                };

                self.chunk.code.push(OpCode::DefEnum(enum_sym, backed_flag));

                for interface in *implements {
                    let interface_str = self.get_text(interface.span);
                    let resolved_iface =
                        self.resolve_ref_name_bytes(interface_str, NameKind::Class);
                    let interface_sym = self.interner.intern(&resolved_iface);
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
                        let resolved = self.resolve_ref_name_bytes(type_name, NameKind::Class);
                        let type_sym = self.interner.intern(&resolved);

                        self.chunk.catch_table.push(CatchEntry {
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
            _ => panic!("Cannot patch non-jump opcode: {:?}", op),
        };
        self.chunk.code[idx] = new_op;
    }

    fn get_literal_value(&self, expr: &Expr) -> Option<Val> {
        match expr {
            Expr::Integer { value, .. } => {
                let s = std::str::from_utf8(value).ok()?;
                let i: i64 = s.parse().ok()?;
                Some(Val::Int(i))
            }
            Expr::String { value, .. } => {
                let s = if value.len() >= 2 {
                    let first = value[0];
                    let last = value[value.len() - 1];
                    if first == b'"' && last == b'"' {
                        let inner = &value[1..value.len() - 1];
                        unescape_string(inner)
                    } else if first == b'\'' && last == b'\'' {
                        let inner = &value[1..value.len() - 1];
                        let mut result = Vec::new();
                        let mut i = 0;
                        while i < inner.len() {
                            if inner[i] == b'\\' && i + 1 < inner.len() {
                                if inner[i + 1] == b'\'' || inner[i + 1] == b'\\' {
                                    result.push(inner[i + 1]);
                                    i += 2;
                                } else {
                                    result.push(inner[i]);
                                    i += 1;
                                }
                            } else {
                                result.push(inner[i]);
                                i += 1;
                            }
                        }
                        result
                    } else {
                        // No quotes - this is from string interpolation (EncapsedAndWhitespace)
                        // These strings need unescaping too
                        unescape_string(value)
                    }
                } else if !value.is_empty() {
                    // Short string without quotes - also from interpolation
                    unescape_string(value)
                } else {
                    value.to_vec()
                };
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
        match expr {
            Expr::Integer { value, .. } => {
                let s = std::str::from_utf8(value).unwrap_or("0");
                let i: i64 = s.parse().unwrap_or(0);
                let idx = self.add_constant(Val::Int(i));
                self.chunk.code.push(OpCode::Const(idx as u16));
            }
            Expr::Float { value, .. } => {
                let s = std::str::from_utf8(value).unwrap_or("0.0");
                let f: f64 = s.parse().unwrap_or(0.0);
                let idx = self.add_constant(Val::Float(f));
                self.chunk.code.push(OpCode::Const(idx as u16));
            }
            Expr::String { value, .. } => {
                let s = if value.len() >= 2 {
                    let first = value[0];
                    let last = value[value.len() - 1];
                    if first == b'"' && last == b'"' {
                        // Double-quoted string: unescape escape sequences
                        let inner = &value[1..value.len() - 1];
                        unescape_string(inner)
                    } else if first == b'\'' && last == b'\'' {
                        // Single-quoted string: no escape processing (except \' and \\)
                        let inner = &value[1..value.len() - 1];
                        let mut result = Vec::new();
                        let mut i = 0;
                        while i < inner.len() {
                            if inner[i] == b'\\' && i + 1 < inner.len() {
                                if inner[i + 1] == b'\'' || inner[i + 1] == b'\\' {
                                    result.push(inner[i + 1]);
                                    i += 2;
                                } else {
                                    result.push(inner[i]);
                                    i += 1;
                                }
                            } else {
                                result.push(inner[i]);
                                i += 1;
                            }
                        }
                        result
                    } else {
                        // No quotes - this is from string interpolation (EncapsedAndWhitespace)
                        // These strings need unescaping too
                        unescape_string(value)
                    }
                } else if !value.is_empty() {
                    // Short string without quotes - also from interpolation
                    unescape_string(value)
                } else {
                    value.to_vec()
                };
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
                        self.emit_expr(left);
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
                                    let resolved =
                                        self.resolve_ref_name_bytes(name, NameKind::Class);
                                    // Bare class name - push as string constant
                                    Val::String(resolved.into())
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
                    UnaryOp::Minus => {
                        // 0 - expr
                        let idx = self.add_constant(Val::Int(0));
                        self.chunk.code.push(OpCode::Const(idx as u16));
                        self.emit_expr(expr);
                        self.chunk.code.push(OpCode::Sub);
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
                                    self.chunk.code.push(OpCode::MakeVarRef(sym));
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
                            Expr::DotAccess { target, property, .. } => {
                                // ++$obj.prop
                                self.emit_expr(target);
                                let prop_name = self.get_text(property.span);
                                let sym = self.interner.intern(prop_name);
                                self.chunk.code.push(OpCode::PreIncDot(sym));
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
                                self.emit_expr(expr);
                            }
                        }
                    }
                    UnaryOp::PreDec => {
                        match expr {
                            Expr::Variable { span, .. } => {
                                let name = self.get_text(*span);
                                if name.starts_with(b"$") {
                                    let sym = self.interner.intern(&name[1..]);
                                    self.chunk.code.push(OpCode::MakeVarRef(sym));
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
                            Expr::DotAccess { target, property, .. } => {
                                // --$obj.prop
                                self.emit_expr(target);
                                let prop_name = self.get_text(property.span);
                                let sym = self.interner.intern(prop_name);
                                self.chunk.code.push(OpCode::PreDecDot(sym));
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
                                self.emit_expr(expr);
                            }
                        }
                    }
                    UnaryOp::ErrorSuppress => {
                        // @ operator: suppress errors for the expression
                        self.chunk.code.push(OpCode::BeginSilence);
                        self.emit_expr(expr);
                        self.chunk.code.push(OpCode::EndSilence);
                    }
                    _ => {
                        self.emit_expr(expr);
                    }
                }
            }
            Expr::PostInc { var, .. } => {
                match var {
                    Expr::Variable { span, .. } => {
                        let name = self.get_text(*span);
                        if name.starts_with(b"$") {
                            let sym = self.interner.intern(&name[1..]);
                            self.chunk.code.push(OpCode::MakeVarRef(sym));
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
                    Expr::DotAccess { target, property, .. } => {
                        // $obj.prop++
                        self.emit_expr(target);
                        let prop_name = self.get_text(property.span);
                        let sym = self.interner.intern(prop_name);
                        self.chunk.code.push(OpCode::PostIncDot(sym));
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
                        // Unsupported post-increment target
                        self.emit_expr(var);
                    }
                }
            }
            Expr::PostDec { var, .. } => {
                match var {
                    Expr::Variable { span, .. } => {
                        let name = self.get_text(*span);
                        if name.starts_with(b"$") {
                            let sym = self.interner.intern(&name[1..]);
                            self.chunk.code.push(OpCode::MakeVarRef(sym));
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
                    Expr::DotAccess { target, property, .. } => {
                        // $obj.prop--
                        self.emit_expr(target);
                        let prop_name = self.get_text(property.span);
                        let sym = self.interner.intern(prop_name);
                        self.chunk.code.push(OpCode::PostDecDot(sym));
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
                        // Unsupported post-decrement target
                        self.emit_expr(var);
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
                                self.emit_expr(array);
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
                                self.emit_expr(target);
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
                            Expr::DotAccess { target, property, .. } => {
                                self.emit_expr(target);
                                let name = self.get_text(property.span);
                                let sym = self.interner.intern(name);
                                self.chunk.code.push(OpCode::IssetDot(sym));
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
                        self.emit_expr(array);
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
                        self.emit_expr(target);
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
                    Expr::DotAccess { target, property, .. } => {
                        self.emit_expr(target);
                        let name = self.get_text(property.span);
                        let sym = self.interner.intern(name);
                        self.chunk.code.push(OpCode::IssetDot(sym));
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
                let mut func_emitter = Emitter::new(self.source, self.interner);
                func_emitter.file_path = self.file_path.clone();
                func_emitter.current_class = self.current_class;
                func_emitter.current_function = Some(closure_sym);
                func_emitter.current_namespace = self.current_namespace;
                func_emitter.use_aliases = self.use_aliases.clone();
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
                            attributes: func_emitter.build_attribute_instances(info.attributes),
                        });

                        if info.variadic {
                            func_emitter
                                .chunk
                                .code
                                .push(OpCode::RecvVariadic(i as u32));
                        } else if let Some(default_expr) = info.default {
                            let val = func_emitter.eval_constant_expr(default_expr);
                            let idx = func_emitter.add_constant(val);
                            func_emitter
                                .chunk
                                .code
                                .push(OpCode::RecvInit(i as u32, idx as u16));
                        } else {
                            func_emitter.chunk.code.push(OpCode::Recv(i as u32));
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
                };

                let func_res = Val::Resource(Rc::new(user_func));
                let const_idx = self.add_constant(func_res);

                self.chunk
                    .code
                    .push(OpCode::Closure(const_idx as u32, use_syms.len() as u32));
            }
            Expr::Call { func, args, .. } => {
                let has_unpack = args.iter().any(|arg| arg.unpack);
                if !has_unpack {
                    match func {
                        Expr::Variable { span, .. } => {
                            let name = self.get_text(*span);
                            if name.starts_with(b"$") {
                                self.emit_expr(func);
                            } else {
                                let resolved = self.resolve_ref_name_bytes(name, NameKind::Function);
                                let idx = self.add_constant(Val::String(resolved.into()));
                                self.chunk.code.push(OpCode::Const(idx as u16));
                            }
                        }
                        _ => self.emit_expr(func),
                    }

                    for arg in *args {
                        self.emit_expr(&arg.value);
                    }

                    self.chunk.code.push(OpCode::Call(args.len() as u8));
                } else {
                    match func {
                        Expr::Variable { span, .. } => {
                            let name = self.get_text(*span);
                            if name.starts_with(b"$") {
                                self.emit_expr(func);
                                self.chunk.code.push(OpCode::InitDynamicCall);
                            } else {
                                let resolved = self.resolve_ref_name_bytes(name, NameKind::Function);
                                let idx = self.add_constant(Val::String(resolved.into()));
                                self.chunk.code.push(OpCode::Const(idx as u16));
                                self.chunk.code.push(OpCode::InitFcallByName);
                            }
                        }
                        _ => {
                            self.emit_expr(func);
                            self.chunk.code.push(OpCode::InitDynamicCall);
                        }
                    }

                    for arg in *args {
                        self.emit_expr(&arg.value);
                        if arg.unpack {
                            self.chunk.code.push(OpCode::SendUnpack);
                        } else {
                            self.chunk.code.push(OpCode::SendFuncArg);
                        }
                    }

                    self.chunk.code.push(OpCode::DoFcall);
                }
            }
            Expr::Variable { span, .. } => {
                let name = self.get_text(*span);
                if name.starts_with(b"$") {
                    let var_name = &name[1..];
                    let sym = self.interner.intern(var_name);
                    self.chunk.code.push(OpCode::LoadVar(sym));
                } else {
                    // Constant fetch
                    let resolved = self.resolve_ref_name_bytes(name, NameKind::Const);
                    let sym = self.interner.intern(&resolved);
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
                    if let Some(key) = item.key {
                        self.emit_expr(key);
                        self.emit_expr(item.value);
                        self.chunk.code.push(OpCode::AssignDim);
                    } else {
                        self.emit_expr(item.value);
                        self.chunk.code.push(OpCode::AppendArray);
                    }
                }
            }
            Expr::ObjectLiteral { items, .. } => {
                self.chunk.code.push(OpCode::InitObjectMap(items.len() as u32));

                for item in *items {
                    let key_bytes = match item.key {
                        ObjectKey::Ident(token) => self.get_text(token.span).to_vec(),
                        ObjectKey::String(token) => {
                            let value = self.get_text(token.span);
                            if value.len() >= 2 {
                                let first = value[0];
                                let last = value[value.len() - 1];
                                if first == b'"' && last == b'"' {
                                    let inner = &value[1..value.len() - 1];
                                    unescape_string(inner)
                                } else if first == b'\'' && last == b'\'' {
                                    let inner = &value[1..value.len() - 1];
                                    let mut result = Vec::new();
                                    let mut i = 0;
                                    while i < inner.len() {
                                        if inner[i] == b'\\' && i + 1 < inner.len() {
                                            if inner[i + 1] == b'\'' || inner[i + 1] == b'\\' {
                                                result.push(inner[i + 1]);
                                                i += 2;
                                            } else {
                                                result.push(inner[i]);
                                                i += 1;
                                            }
                                        } else {
                                            result.push(inner[i]);
                                            i += 1;
                                        }
                                    }
                                    result
                                } else {
                                    unescape_string(value)
                                }
                            } else if !value.is_empty() {
                                unescape_string(value)
                            } else {
                                value.to_vec()
                            }
                        }
                    };

                    let prop_sym = self.interner.intern(&key_bytes);
                    self.chunk.code.push(OpCode::Dup);
                    self.emit_expr(item.value);
                    self.chunk.code.push(OpCode::AssignDot(prop_sym));
                    self.chunk.code.push(OpCode::Pop);
                }
            }
            Expr::JsxElement {
                name,
                attributes,
                children,
                ..
            } => {
                self.emit_jsx_call(Some(name), attributes, children);
            }
            Expr::JsxFragment { children, .. } => {
                self.emit_jsx_call(None, &[], children);
            }
            Expr::StructLiteral { name, fields, .. } => {
                let name_bytes = self.get_text(name.span);
                let resolved = self.resolve_ref_name_bytes(name_bytes, NameKind::Class);
                let class_sym = self.interner.intern(&resolved);
                self.chunk.code.push(OpCode::NewStructLiteral(class_sym));

                for field in *fields {
                    let field_bytes = self.get_text(field.name.span);
                    let field_name = if field_bytes.starts_with(b"$") {
                        &field_bytes[1..]
                    } else {
                        field_bytes
                    };
                    let prop_sym = self.interner.intern(field_name);
                    self.chunk.code.push(OpCode::Dup);
                    self.emit_expr(field.value);
                    self.chunk.code.push(OpCode::AssignDot(prop_sym));
                self.chunk.code.push(OpCode::Pop);
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
                if let Expr::Variable { span, .. } = class {
                    let name = self.get_text(*span);
                    if !name.starts_with(b"$") {
                        let resolved = self.resolve_ref_name_bytes(name, NameKind::Class);
                        let class_sym = self.interner.intern(&resolved);

                        for arg in *args {
                            self.emit_expr(arg.value);
                        }

                        self.chunk
                            .code
                            .push(OpCode::New(class_sym, args.len() as u8));
                    } else {
                        // Dynamic new $var()
                        // Emit expression to get class name (string)
                        self.emit_expr(class);

                        for arg in *args {
                            self.emit_expr(arg.value);
                        }

                        self.chunk.code.push(OpCode::NewDynamic(args.len() as u8));
                    }
                } else {
                    // Complex expression for class name
                    self.emit_expr(class);

                    for arg in *args {
                        self.emit_expr(arg.value);
                    }

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
            Expr::DotAccess { target, property, .. } => {
                self.emit_expr(target);
                let name = self.get_text(property.span);
                let sym = self.interner.intern(name);
                self.chunk.code.push(OpCode::FetchDot(sym));
            }
            Expr::MethodCall {
                target,
                method,
                args,
                ..
            } => {
                let has_unpack = args.iter().any(|arg| arg.unpack);
                if !has_unpack {
                    self.emit_expr(target);
                    if let Expr::Variable { span, .. } = method {
                        let name = self.get_text(*span);
                        if !name.starts_with(b"$") {
                            for arg in *args {
                                self.emit_expr(arg.value);
                            }
                            let sym = self.interner.intern(name);
                            self.chunk
                                .code
                                .push(OpCode::CallMethod(sym, args.len() as u8));
                        } else {
                            // Dynamic method call: $obj->$method()
                            self.emit_expr(method);
                            for arg in *args {
                                self.emit_expr(arg.value);
                            }
                            self.chunk
                                .code
                                .push(OpCode::CallMethodDynamic(args.len() as u8));
                        }
                    } else {
                        // Dynamic method call with expression: $obj->{$expr}()
                        self.emit_expr(method);
                        for arg in *args {
                            self.emit_expr(arg.value);
                        }
                        self.chunk
                            .code
                            .push(OpCode::CallMethodDynamic(args.len() as u8));
                    }
                } else {
                    self.emit_expr(target);
                    if let Expr::Variable { span, .. } = method {
                        let name = self.get_text(*span);
                        if !name.starts_with(b"$") {
                            let idx = self.add_constant(Val::String(name.to_vec().into()));
                            self.chunk.code.push(OpCode::Const(idx as u16));
                        } else {
                            self.emit_expr(method);
                        }
                    } else {
                        self.emit_expr(method);
                    }
                    self.chunk.code.push(OpCode::InitMethodCall);
                    for arg in *args {
                        self.emit_expr(arg.value);
                        if arg.unpack {
                            self.chunk.code.push(OpCode::SendUnpack);
                        } else {
                            self.chunk.code.push(OpCode::SendFuncArg);
                        }
                    }
                    self.chunk.code.push(OpCode::DoFcall);
                }
            }
            Expr::StaticCall {
                class,
                method,
                args,
                ..
            } => {
                let has_unpack = args.iter().any(|arg| arg.unpack);
                if !has_unpack {
                    let mut class_emitted = false;
                    if let Expr::Variable { span, .. } = class {
                        let class_name = self.get_text(*span);
                        if !class_name.starts_with(b"$") {
                            let resolved = self.resolve_ref_name_bytes(class_name, NameKind::Class);
                            let class_sym = self.interner.intern(&resolved);

                            if let Expr::Variable {
                                span: method_span, ..
                            } = method
                            {
                                let method_name = self.get_text(*method_span);
                                if !method_name.starts_with(b"$") {
                                    for arg in *args {
                                        self.emit_expr(arg.value);
                                    }
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
                                let idx = self.add_constant(Val::String(resolved.into()));
                                self.chunk.code.push(OpCode::Const(idx as u16));
                                self.emit_expr(method);
                                for arg in *args {
                                    self.emit_expr(arg.value);
                                }
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
                        for arg in *args {
                            self.emit_expr(arg.value);
                        }
                        self.chunk
                            .code
                            .push(OpCode::CallStaticMethodDynamic(args.len() as u8));
                    }
                } else {
                    let mut class_pushed = false;
                    if let Expr::Variable { span, .. } = class {
                        let class_name = self.get_text(*span);
                        if !class_name.starts_with(b"$") {
                            let resolved = self.resolve_ref_name_bytes(class_name, NameKind::Class);
                            let idx = self.add_constant(Val::String(resolved.into()));
                            self.chunk.code.push(OpCode::Const(idx as u16));
                            class_pushed = true;
                        }
                    }
                    if !class_pushed {
                        self.emit_expr(class);
                    }

                    if let Expr::Variable { span, .. } = method {
                        let method_name = self.get_text(*span);
                        if !method_name.starts_with(b"$") {
                            let idx = self.add_constant(Val::String(method_name.to_vec().into()));
                            self.chunk.code.push(OpCode::Const(idx as u16));
                        } else {
                            self.emit_expr(method);
                        }
                    } else {
                        self.emit_expr(method);
                    }

                    self.chunk.code.push(OpCode::InitStaticMethodCall);

                    for arg in *args {
                        self.emit_expr(arg.value);
                        if arg.unpack {
                            self.chunk.code.push(OpCode::SendUnpack);
                        } else {
                            self.chunk.code.push(OpCode::SendFuncArg);
                        }
                    }

                    self.chunk.code.push(OpCode::DoFcall);
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
                            let resolved = self.resolve_ref_name_bytes(class_name, NameKind::Class);
                            let idx = self.add_constant(Val::String(resolved.into()));
                            self.chunk.code.push(OpCode::Const(idx as u16));
                            return;
                        }

                        let resolved = self.resolve_ref_name_bytes(class_name, NameKind::Class);
                        let class_sym = self.interner.intern(&resolved);

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
                    self.emit_expr(expr);
                    if let Expr::Variable { span, .. } = property {
                        let name = self.get_text(*span);
                        if !name.starts_with(b"$") {
                            let sym = self.interner.intern(name);
                            self.chunk.code.push(OpCode::AssignProp(sym));
                        }
                    }
                }
                Expr::DotAccess { target, property, .. } => {
                    self.emit_expr(target);
                    self.emit_expr(expr);
                    let name = self.get_text(property.span);
                    let sym = self.interner.intern(name);
                    self.chunk.code.push(OpCode::AssignDot(sym));
                }
                Expr::ClassConstFetch {
                    class, constant, ..
                } => {
                    self.emit_expr(expr);
                    if let Expr::Variable { span, .. } = class {
                        let class_name = self.get_text(*span);
                        if !class_name.starts_with(b"$") {
                            let resolved = self.resolve_ref_name_bytes(class_name, NameKind::Class);
                            let class_sym = self.interner.intern(&resolved);

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
                Expr::ArrayDimFetch { .. } => {
                    let (base, keys) = Self::flatten_dim_fetch(var);

                    if let Expr::PropertyFetch {
                        target, property, ..
                    } = base
                    {
                        self.emit_expr(target);
                        self.chunk.code.push(OpCode::Dup);

                        if let Expr::Variable { span, .. } = property {
                            let name = self.get_text(*span);
                            if !name.starts_with(b"$") {
                                let sym = self.interner.intern(name);
                                self.chunk.code.push(OpCode::FetchProp(sym));

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
                                    .push(OpCode::StoreNestedDim(keys.len() as u8));

                                self.chunk.code.push(OpCode::AssignProp(sym));
                            }
                        }
                    } else if let Expr::DotAccess { target, property, .. } = base {
                        self.emit_expr(target);
                        self.chunk.code.push(OpCode::Dup);

                        let name = self.get_text(property.span);
                        let sym = self.interner.intern(name);
                        self.chunk.code.push(OpCode::FetchDot(sym));

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
                            .push(OpCode::StoreNestedDim(keys.len() as u8));

                        self.chunk.code.push(OpCode::AssignDot(sym));
                    } else {
                        self.emit_expr(base);
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
                            .push(OpCode::StoreNestedDim(keys.len() as u8));

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
                }
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
                            self.emit_expr(expr);
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
                        self.emit_expr(array_var);
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
                            self.emit_expr(expr);
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
                    _ => {
                        // TODO: Support other targets for reference assignment
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
                    Expr::DotAccess { target, property, .. } => {
                        self.emit_expr(target);
                        self.chunk.code.push(OpCode::Dup);

                        let name = self.get_text(property.span);
                        let sym = self.interner.intern(name);

                        if let AssignOp::Coalesce = op {
                            self.chunk.code.push(OpCode::Dup);
                            self.chunk.code.push(OpCode::IssetDot(sym));
                            let jump_idx = self.chunk.code.len();
                            self.chunk.code.push(OpCode::JmpIfTrue(0));

                            self.emit_expr(expr);
                            self.chunk.code.push(OpCode::AssignDot(sym));

                            let end_jump_idx = self.chunk.code.len();
                            self.chunk.code.push(OpCode::Jmp(0));

                            let label_set = self.chunk.code.len();
                            self.chunk.code[jump_idx] = OpCode::JmpIfTrue(label_set as u32);
                            self.chunk.code.push(OpCode::FetchDot(sym));

                            let label_end = self.chunk.code.len();
                            self.chunk.code[end_jump_idx] = OpCode::Jmp(label_end as u32);
                            return;
                        }

                        self.chunk.code.push(OpCode::FetchDot(sym));

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

                        self.chunk.code.push(OpCode::AssignDot(sym));
                    }
                    Expr::ClassConstFetch {
                        class, constant, ..
                    } => {
                        if let Expr::Variable { span, .. } = class {
                            let class_name = self.get_text(*span);
                            if !class_name.starts_with(b"$") {
                                let resolved =
                                    self.resolve_ref_name_bytes(class_name, NameKind::Class);
                                let class_sym = self.interner.intern(&resolved);

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
                                                resolved.clone().into(),
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
            Expr::Integer { value, .. } => {
                let s_str = std::str::from_utf8(value).unwrap_or("0");
                if let Ok(i) = s_str.parse::<i64>() {
                    Val::Int(i)
                } else {
                    Val::Int(0)
                }
            }
            Expr::Float { value, .. } => {
                let s_str = std::str::from_utf8(value).unwrap_or("0.0");
                if let Ok(f) = s_str.parse::<f64>() {
                    Val::Float(f)
                } else {
                    Val::Float(0.0)
                }
            }
            Expr::String { value, .. } => {
                let s = value;
                if s.len() >= 2
                    && ((s[0] == b'"' && s[s.len() - 1] == b'"')
                        || (s[0] == b'\'' && s[s.len() - 1] == b'\''))
                {
                    Val::String(s[1..s.len() - 1].to_vec().into())
                } else {
                    Val::String(s.to_vec().into())
                }
            }
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
            Expr::ClassConstFetch {
                class, constant, ..
            } => {
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
            Expr::Binary {
                left, op, right, ..
            } => match op {
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
            },
            _ => Val::Null,
        }
    }

    fn emit_jsx_call(
        &mut self,
        name: Option<&Name<'_>>,
        attributes: &[JsxAttribute<'_>],
        children: &[JsxChild<'_>],
    ) {
        let child_count = self.jsx_child_count(children);
        let (key_expr, prop_count) = self.jsx_prop_count(attributes, child_count);

        let fn_name: &[u8] = if child_count > 1 { b"jsxs" } else { b"jsx" };
        let resolved = self.resolve_ref_name_bytes(fn_name, NameKind::Function);
        let fn_idx = self.add_constant(Val::String(resolved.into()));
        self.chunk.code.push(OpCode::Const(fn_idx as u16));

        self.emit_jsx_type(name);
        self.emit_jsx_props(attributes, children, prop_count, child_count > 0);

        if let Some(expr) = key_expr {
            self.emit_expr(expr);
            self.chunk.code.push(OpCode::Call(3));
        } else {
            self.chunk.code.push(OpCode::Call(2));
        }
    }

    fn emit_jsx_type(&mut self, name: Option<&Name<'_>>) {
        let tag_bytes = if let Some(name) = name {
            let raw = self.get_text(name.span);
            let trimmed = if raw.starts_with(b"\\") { &raw[1..] } else { raw };
            let mut is_component = false;
            if let Some(last_seg) = trimmed.split(|b| *b == b'\\').last() {
                if let Some(first) = last_seg.first() {
                    is_component = *first >= b'A' && *first <= b'Z';
                }
            }
            if is_component {
                let mut resolved = self.resolve_ref_name_bytes(raw, NameKind::Function);
                if let Some(pos) = resolved.iter().position(|b| *b == 0) {
                    resolved.truncate(pos);
                }
                resolved
            } else {
                trimmed.to_vec()
            }
        } else {
            b"__fragment__".to_vec()
        };
        let idx = self.add_constant(Val::String(tag_bytes.into()));
        self.chunk.code.push(OpCode::Const(idx as u16));
    }

    fn emit_jsx_props(
        &mut self,
        attributes: &[JsxAttribute<'_>],
        children: &[JsxChild<'_>],
        prop_count: usize,
        include_children: bool,
    ) {
        self.chunk
            .code
            .push(OpCode::InitObjectMap(prop_count as u32));

        for attr in attributes {
            if self.is_jsx_key(attr) {
                continue;
            }
            let name_bytes = self.get_text(attr.name.span);
            let prop_sym = self.interner.intern(name_bytes);
            self.chunk.code.push(OpCode::Dup);
            if let Some(value) = attr.value {
                self.emit_expr(value);
            } else {
                let idx = self.add_constant(Val::Bool(true));
                self.chunk.code.push(OpCode::Const(idx as u16));
            }
            self.chunk.code.push(OpCode::AssignDot(prop_sym));
            self.chunk.code.push(OpCode::Pop);
        }

        if include_children {
            let child_sym = self.interner.intern(b"children");
            self.chunk.code.push(OpCode::Dup);
            self.emit_jsx_children_array(children);
            self.chunk.code.push(OpCode::AssignDot(child_sym));
            self.chunk.code.push(OpCode::Pop);
        }
    }

    fn emit_jsx_children_array(&mut self, children: &[JsxChild<'_>]) {
        let child_count = self.jsx_child_count(children);
        self.chunk
            .code
            .push(OpCode::InitArray(child_count as u32));

        for &child in children {
            match child {
                JsxChild::Text(span) => {
                    if let Some(text) = self.normalize_jsx_text(span) {
                        let idx = self.add_constant(Val::String(text.into()));
                        self.chunk.code.push(OpCode::Const(idx as u16));
                        self.chunk.code.push(OpCode::AppendArray);
                    }
                }
                JsxChild::Expr(expr) => {
                    self.emit_expr(expr);
                    self.chunk.code.push(OpCode::AppendArray);
                }
            }
        }
    }

    fn normalize_jsx_text(&self, span: Span) -> Option<Vec<u8>> {
        let raw = self.get_text(span);
        let text = String::from_utf8_lossy(raw);
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.as_bytes().to_vec())
        }
    }

    fn jsx_child_count(&self, children: &[JsxChild<'_>]) -> usize {
        let mut count = 0;
        for &child in children {
            match child {
                JsxChild::Text(span) => {
                    if self.normalize_jsx_text(span).is_some() {
                        count += 1;
                    }
                }
                JsxChild::Expr(_) => count += 1,
            }
        }
        count
    }

    fn jsx_prop_count<'a>(
        &self,
        attributes: &[JsxAttribute<'a>],
        child_count: usize,
    ) -> (Option<ExprId<'a>>, usize) {
        let mut key_expr = None;
        let mut count = 0;
        for attr in attributes {
            if self.is_jsx_key(attr) {
                if key_expr.is_none() {
                    key_expr = attr.value;
                }
                continue;
            }
            count += 1;
        }
        if child_count > 0 {
            count += 1;
        }
        (key_expr, count)
    }

    fn is_jsx_key(&self, attr: &JsxAttribute<'_>) -> bool {
        let name_bytes = self.get_text(attr.name.span);
        name_bytes.eq_ignore_ascii_case(b"key")
    }

    fn get_text(&self, span: crate::parser::span::Span) -> &'src [u8] {
        &self.source[span.start..span.end]
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
                let resolved = self.resolve_ref_name_bytes(class_name, NameKind::Class);
                let class_idx = self.add_constant(Val::String(Rc::new(resolved)));
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

    fn current_namespace_bytes(&self) -> Option<&[u8]> {
        self.current_namespace
            .and_then(|sym| self.interner.lookup(sym))
    }

    fn normalize_alias_key(bytes: &[u8]) -> Vec<u8> {
        bytes.iter().map(|b| b.to_ascii_lowercase()).collect()
    }

    fn split_name_bytes(name: &[u8]) -> (bool, Vec<Vec<u8>>) {
        let mut raw = name;
        let mut is_absolute = false;
        if raw.starts_with(b"\\") {
            is_absolute = true;
            raw = &raw[1..];
        }
        let parts = raw
            .split(|b| *b == b'\\')
            .filter(|p| !p.is_empty())
            .map(|p| p.to_vec())
            .collect::<Vec<_>>();
        (is_absolute, parts)
    }

    fn join_name_parts(parts: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        for (idx, part) in parts.iter().enumerate() {
            if idx > 0 {
                out.push(b'\\');
            }
            out.extend_from_slice(part);
        }
        out
    }

    fn is_namespace_keyword(part: &[u8]) -> bool {
        part.eq_ignore_ascii_case(b"namespace")
    }

    fn resolve_decl_name_bytes(&self, name: &[u8]) -> Vec<u8> {
        let (is_absolute, mut parts) = Self::split_name_bytes(name);
        if parts.is_empty() {
            return Vec::new();
        }
        if is_absolute {
            return Self::join_name_parts(&parts);
        }
        if Self::is_namespace_keyword(&parts[0]) {
            parts.remove(0);
            if let Some(ns) = self.current_namespace_bytes() {
                if parts.is_empty() {
                    return ns.to_vec();
                }
                let mut out = Vec::new();
                if !ns.is_empty() {
                    out.extend_from_slice(ns);
                    out.push(b'\\');
                }
                out.extend_from_slice(&Self::join_name_parts(&parts));
                return out;
            }
        }
        if let Some(ns) = self.current_namespace_bytes() {
            if ns.is_empty() {
                return Self::join_name_parts(&parts);
            }
            let mut out = Vec::new();
            out.extend_from_slice(ns);
            out.push(b'\\');
            out.extend_from_slice(&Self::join_name_parts(&parts));
            return out;
        }
        Self::join_name_parts(&parts)
    }

    fn normalize_namespace_decl(&self, name: &[u8]) -> Vec<u8> {
        let (_is_absolute, mut parts) = Self::split_name_bytes(name);
        if parts.is_empty() {
            return Vec::new();
        }
        if Self::is_namespace_keyword(&parts[0]) {
            parts.remove(0);
        }
        Self::join_name_parts(&parts)
    }

    fn resolve_ref_name_bytes(&self, name: &[u8], kind: NameKind) -> Vec<u8> {
        let (is_absolute, mut parts) = Self::split_name_bytes(name);
        if parts.is_empty() {
            return Vec::new();
        }
        let is_unqualified = !is_absolute
            && parts.len() == 1
            && !Self::is_namespace_keyword(&parts[0]);
        if is_absolute {
            return Self::join_name_parts(&parts);
        }

        if kind == NameKind::Class
            && parts.len() == 1
            && (parts[0].eq_ignore_ascii_case(b"self")
                || parts[0].eq_ignore_ascii_case(b"parent")
                || parts[0].eq_ignore_ascii_case(b"static"))
        {
            return parts[0].clone();
        }

        if Self::is_namespace_keyword(&parts[0]) {
            parts.remove(0);
            if let Some(ns) = self.current_namespace_bytes() {
                if parts.is_empty() {
                    return ns.to_vec();
                }
                let mut out = Vec::new();
                if !ns.is_empty() {
                    out.extend_from_slice(ns);
                    out.push(b'\\');
                }
                out.extend_from_slice(&Self::join_name_parts(&parts));
                return out;
            }
        }

        let alias_key = Self::normalize_alias_key(&parts[0]);
        let alias_target = match kind {
            NameKind::Class => self.use_aliases.classes.get(&alias_key),
            NameKind::Function => self.use_aliases.functions.get(&alias_key),
            NameKind::Const => self.use_aliases.consts.get(&alias_key),
        };
        if let Some(target) = alias_target {
            if parts.len() == 1 {
                return target.clone();
            }
            let mut out = target.clone();
            if !out.is_empty() {
                out.push(b'\\');
            }
            let rest = Self::join_name_parts(&parts[1..]);
            out.extend_from_slice(&rest);
            return out;
        }

        if matches!(kind, NameKind::Function | NameKind::Const) && is_unqualified {
            if let Some(ns) = self.current_namespace_bytes() {
                if !ns.is_empty() {
                    let mut out = Vec::new();
                    out.extend_from_slice(ns);
                    out.push(b'\\');
                    out.extend_from_slice(&parts[0]);
                    // Encode fallback: "<ns>\\name\0name"
                    out.push(0);
                    out.extend_from_slice(&parts[0]);
                    return out;
                }
            }
            return parts[0].clone();
        }

        if let Some(ns) = self.current_namespace_bytes() {
            if ns.is_empty() {
                return Self::join_name_parts(&parts);
            }
            let mut out = Vec::new();
            out.extend_from_slice(ns);
            out.push(b'\\');
            out.extend_from_slice(&Self::join_name_parts(&parts));
            return out;
        }

        Self::join_name_parts(&parts)
    }

    fn reset_use_aliases(&mut self) {
        self.use_aliases = UseAliases::default();
    }

    fn register_use_item(&mut self, item: &UseItem) {
        let raw_name = self.get_text(item.name.span);
        let (is_absolute, mut parts) = Self::split_name_bytes(raw_name);
        if parts.is_empty() {
            return;
        }

        // Use statements are absolute by default; namespace\Foo resolves to current namespace.
        if !is_absolute && Self::is_namespace_keyword(&parts[0]) {
            parts.remove(0);
            if let Some(ns) = self.current_namespace_bytes() {
                let mut prefixed = Vec::new();
                if !ns.is_empty() {
                    prefixed.extend_from_slice(ns);
                    prefixed.push(b'\\');
                }
                prefixed.extend_from_slice(&Self::join_name_parts(&parts));
                parts = Self::split_name_bytes(&prefixed).1;
            }
        }

        let target = Self::join_name_parts(&parts);
        if target.is_empty() {
            return;
        }

        let alias = if let Some(alias_tok) = item.alias {
            self.get_text(alias_tok.span).to_vec()
        } else {
            parts.last().cloned().unwrap_or_default()
        };
        if alias.is_empty() {
            return;
        }
        let key = Self::normalize_alias_key(&alias);
        match item.kind {
            UseKind::Normal => {
                self.use_aliases.classes.insert(key, target);
            }
            UseKind::Function => {
                self.use_aliases.functions.insert(key, target);
            }
            UseKind::Const => {
                self.use_aliases.consts.insert(key, target);
            }
        }
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
                let resolved = self.resolve_ref_name_bytes(name_str, NameKind::Class);
                let sym = self.interner.intern(&resolved);
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
            Type::ObjectShape(_) => None,
            Type::Applied { .. } => None,
        }
    }
}

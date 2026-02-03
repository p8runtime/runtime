use crate::parser::ast::{
    BinaryOp, ClassKind, ClassMember, Expr, ExprId, JsxChild, Name, ObjectKey, Program,
    PropertyEntry, Stmt, Type as AstType, TypeParam, UnaryOp,
};
use crate::parser::ast::visitor::{walk_expr, Visitor};
use crate::parser::lexer::token::TokenKind;
use crate::parser::span::Span;
use crate::phpx::typeck::infer::{
    infer_expr, EnumCaseInfo, EnumInfo, EnumParamInfo, InferContext, StructInfo,
};
use crate::phpx::typeck::types::{merge_types, ObjectField, PrimitiveType, Type};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone)]
struct ParamSig {
    ty: Option<Type>,
    required: bool,
}

#[derive(Debug, Clone)]
struct TypeParamSig {
    name: String,
    constraint: Option<Type>,
}

#[derive(Debug, Clone)]
struct FunctionSig {
    type_params: Vec<TypeParamSig>,
    params: Vec<ParamSig>,
    return_type: Option<Type>,
    variadic: bool,
}

#[derive(Debug, Clone)]
struct MethodSig {
    params: Vec<ParamSig>,
    return_type: Option<Type>,
    variadic: bool,
}

#[derive(Debug, Clone)]
struct InterfaceInfo {
    methods: HashMap<String, MethodSig>,
}

#[derive(Debug, Clone)]
struct TypeAliasInfo {
    params: Vec<TypeParamSig>,
    ty: Type,
    span: Span,
}

#[derive(Debug, Clone)]
enum StructFieldResolution {
    Found(Type),
    Missing,
    Ambiguous,
}

#[derive(Debug, Clone)]
pub struct TypeError {
    pub span: Span,
    pub message: String,
}

struct JsxExprValidator {
    errors: Vec<TypeError>,
}

impl<'ast> Visitor<'ast> for JsxExprValidator {
    fn visit_expr(&mut self, expr: ExprId<'ast>) {
        match *expr {
            Expr::Assign { span, .. }
            | Expr::AssignRef { span, .. }
            | Expr::AssignOp { span, .. } => {
                self.errors.push(TypeError {
                    span,
                    message: "Statements not allowed in JSX expressions".to_string(),
                });
            }
            Expr::Yield { span, .. } => {
                self.errors.push(TypeError {
                    span,
                    message: "Statements not allowed in JSX expressions".to_string(),
                });
            }
            Expr::Error { span } => {
                self.errors.push(TypeError {
                    span,
                    message: "Invalid JSX expression".to_string(),
                });
            }
            _ => {}
        }
        walk_expr(self, expr);
    }
}

impl TypeError {
    pub fn to_human_readable(&self, source: &[u8]) -> String {
        let Some(info) = self.span.line_info(source) else {
            return format!("type error: {}", self.message);
        };
        let line_str = String::from_utf8_lossy(info.line_text);
        let gutter_width = info.line.to_string().len();
        let padding = std::cmp::min(info.line_text.len(), info.column.saturating_sub(1));
        let highlight_len = std::cmp::max(
            1,
            std::cmp::min(self.span.len(), info.line_text.len().saturating_sub(padding)),
        );

        let mut marker = String::new();
        marker.push_str(&" ".repeat(padding));
        marker.push_str(&"^".repeat(highlight_len));

        format!(
            "type error: {}\n --> line {}, column {}\n{gutter}|\n{line_no:>width$} | {line_src}\n{gutter}| {marker}",
            self.message,
            info.line,
            info.column,
            gutter = " ".repeat(gutter_width + 1),
            line_no = info.line,
            width = gutter_width,
            line_src = line_str,
            marker = marker,
        )
    }
}

pub fn check_program(program: &Program, source: &[u8]) -> Result<(), Vec<TypeError>> {
    check_program_with_path(program, source, None)
}

pub fn check_program_with_path(
    program: &Program,
    source: &[u8],
    file_path: Option<&Path>,
) -> Result<(), Vec<TypeError>> {
    let mut ctx = CheckContext::new(source, file_path);
    ctx.check_program(program);

    if ctx.errors.is_empty() {
        Ok(())
    } else {
        Err(ctx.errors)
    }
}

pub fn format_type_errors(errors: &[TypeError], source: &[u8]) -> String {
    let mut out = String::new();
    for (idx, err) in errors.iter().enumerate() {
        if idx > 0 {
            out.push('\n');
        }
        out.push_str(&err.to_human_readable(source));
    }
    out
}

struct CheckContext<'a> {
    #[allow(dead_code)]
    source: &'a [u8],
    file_path: Option<PathBuf>,
    errors: Vec<TypeError>,
    structs: HashMap<String, StructInfo>,
    struct_methods: HashMap<String, HashMap<String, MethodSig>>,
    enums: HashMap<String, EnumInfo>,
    enum_methods: HashMap<String, HashMap<String, MethodSig>>,
    interfaces: HashMap<String, InterfaceInfo>,
    functions: HashMap<String, FunctionSig>,
    function_returns: HashMap<String, Type>,
    type_aliases: HashMap<String, TypeAliasInfo>,
    resolved_aliases: HashMap<String, Type>,
}

impl<'a> CheckContext<'a> {
    fn new(source: &'a [u8], file_path: Option<&Path>) -> Self {
        Self {
            source,
            file_path: file_path.map(|path| path.to_path_buf()),
            errors: Vec::new(),
            structs: HashMap::new(),
            struct_methods: HashMap::new(),
            enums: HashMap::new(),
            enum_methods: HashMap::new(),
            interfaces: HashMap::new(),
            functions: HashMap::new(),
            function_returns: HashMap::new(),
            type_aliases: HashMap::new(),
            resolved_aliases: HashMap::new(),
        }
    }

    fn check_program(&mut self, program: &Program<'a>) {
        self.check_wasm_stubs();
        self.collect_struct_names(program);
        self.collect_interface_names(program);
        self.collect_enum_names(program);
        self.collect_type_aliases(program);
        self.collect_struct_fields(program);
        self.collect_interface_methods(program);
        self.collect_struct_methods(program);
        self.collect_enum_methods(program);
        self.collect_enum_cases(program);
        self.collect_functions(program);
        let mut env: HashMap<String, Type> = HashMap::new();
        let mut explicit: HashSet<String> = HashSet::new();
        for stmt in program.statements.iter() {
            self.check_stmt(stmt, &mut env, &mut explicit, None);
        }
    }

    fn validate_jsx_expr(&mut self, expr: ExprId<'a>) {
        let mut validator = JsxExprValidator { errors: Vec::new() };
        validator.visit_expr(expr);
        self.errors.extend(validator.errors);
    }

    fn check_stmt(
        &mut self,
        stmt: &Stmt<'a>,
        env: &mut HashMap<String, Type>,
        explicit: &mut HashSet<String>,
        return_type: Option<&Type>,
    ) {
        match stmt {
            Stmt::Return { expr, span } => {
                if let Some(expected) = return_type {
                    let actual = expr
                        .map(|expr| self.check_expr(expr, env, explicit))
                        .unwrap_or_else(|| Type::Primitive(PrimitiveType::Null));
                    if let Some(expr) = expr {
                        if let Expr::Null { span: null_span } = *expr {
                            if !self.type_allows_null(expected) {
                                self.errors.push(TypeError {
                                    span: *null_span,
                                    message: "Null is not allowed in PHPX; use Option<T> instead"
                                        .to_string(),
                                });
                            }
                        }
                    } else if !self.type_allows_null(expected) {
                        self.errors.push(TypeError {
                            span: *span,
                            message: "Null is not allowed in PHPX; use Option<T> instead"
                                .to_string(),
                        });
                    }
                    if let Some(expr) = expr {
                        if let Expr::ObjectLiteral { items, span: obj_span } = *expr {
                            self.check_object_literal_against_type(
                                items,
                                expected,
                                *obj_span,
                                env,
                            );
                        }
                    }
                    if !self.is_assignable(&actual, expected) {
                        self.errors.push(TypeError {
                            span: *span,
                            message: format!(
                                "Return type mismatch: expected {}, got {}",
                                expected, actual
                            ),
                        });
                    }
                }
                if return_type.is_none() {
                    if let Some(expr) = expr {
                        if let Expr::Null { span: null_span } = *expr {
                            self.errors.push(TypeError {
                                span: *null_span,
                                message: "Null is not allowed in PHPX; use Option<T> instead"
                                    .to_string(),
                            });
                        }
                    }
                }
            }
            Stmt::Expression { expr, .. } => {
                if let Expr::Null { span } = *expr {
                    self.errors.push(TypeError {
                        span: *span,
                        message: "Null is not allowed in PHPX; use Option<T> instead".to_string(),
                    });
                }
                let _ = self.check_expr(expr, env, explicit);
            }
            Stmt::If {
                condition,
                then_block,
                else_block,
                ..
            } => {
                let _ = self.check_expr(condition, env, explicit);
                let mut then_env = self.narrow_env_for_condition(condition, env, true);
                let mut then_explicit = explicit.clone();
                for stmt in then_block.iter() {
                    self.check_stmt(stmt, &mut then_env, &mut then_explicit, return_type);
                }
                if let Some(else_block) = else_block {
                    let mut else_env = self.narrow_env_for_condition(condition, env, false);
                    let mut else_explicit = explicit.clone();
                    for stmt in else_block.iter() {
                        self.check_stmt(stmt, &mut else_env, &mut else_explicit, return_type);
                    }
                }
            }
            Stmt::While { condition, body, .. } => {
                let _ = self.check_expr(condition, env, explicit);
                let mut loop_env = self.narrow_env_for_condition(condition, env, true);
                let mut loop_explicit = explicit.clone();
                for stmt in body.iter() {
                    self.check_stmt(stmt, &mut loop_env, &mut loop_explicit, return_type);
                }
            }
            Stmt::DoWhile {
                body, condition, ..
            } => {
                let mut loop_env = env.clone();
                let mut loop_explicit = explicit.clone();
                for stmt in body.iter() {
                    self.check_stmt(stmt, &mut loop_env, &mut loop_explicit, return_type);
                }
                let _ = self.check_expr(condition, env, explicit);
            }
            Stmt::For {
                init,
                condition,
                loop_expr,
                body,
                ..
            } => {
                for expr in init.iter() {
                    let _ = self.check_expr(expr, env, explicit);
                }
                for expr in condition.iter() {
                    let _ = self.check_expr(expr, env, explicit);
                }
                for expr in loop_expr.iter() {
                    let _ = self.check_expr(expr, env, explicit);
                }
                let mut loop_env = if condition.len() == 1 {
                    self.narrow_env_for_condition(condition[0], env, true)
                } else {
                    env.clone()
                };
                let mut loop_explicit = explicit.clone();
                for stmt in body.iter() {
                    self.check_stmt(stmt, &mut loop_env, &mut loop_explicit, return_type);
                }
            }
            Stmt::Foreach { expr, body, .. } => {
                let _ = self.check_expr(expr, env, explicit);
                let mut loop_env = env.clone();
                let mut loop_explicit = explicit.clone();
                for stmt in body.iter() {
                    self.check_stmt(stmt, &mut loop_env, &mut loop_explicit, return_type);
                }
            }
            Stmt::Block { statements, .. } => {
                let mut block_env = env.clone();
                let mut block_explicit = explicit.clone();
                for stmt in statements.iter() {
                    self.check_stmt(stmt, &mut block_env, &mut block_explicit, return_type);
                }
            }
            Stmt::Function {
                type_params,
                params,
                return_type: fn_return,
                body,
                ..
            } => {
                let (type_param_sigs, type_param_set) =
                    self.collect_type_param_sigs(type_params);
                let mut fn_env: HashMap<String, Type> = HashMap::new();
                let mut fn_explicit: HashSet<String> = HashSet::new();
                for param in params.iter() {
                    if let Some(ty) = param.ty {
                        let param_name = token_text(self.source, param.name.span);
                        let param_name = param_name.trim_start_matches('$').to_string();
                        let resolved = self.resolve_type_with_params(ty, &type_param_set);
                        fn_env.insert(param_name.clone(), resolved);
                        fn_explicit.insert(param_name);
                    }
                    if let Some(default) = param.default {
                        if let Some(ty) = param.ty {
                            let expected = self.resolve_type_with_params(ty, &type_param_set);
                            let actual = self.check_expr(default, env, explicit);
                            if !self.is_assignable(&actual, &expected) {
                                self.errors.push(TypeError {
                                    span: param.span,
                                    message: format!(
                                        "Default parameter type mismatch: expected {}, got {}",
                                        expected, actual
                                    ),
                                });
                            }
                        }
                    }
                }
                let expected_return =
                    fn_return.map(|ty| self.resolve_type_with_params(ty, &type_param_set));
                for stmt in body.iter() {
                    self.check_stmt(
                        stmt,
                        &mut fn_env,
                        &mut fn_explicit,
                        expected_return.as_ref(),
                    );
                }

                let _ = type_param_sigs;
            }
            Stmt::Class {
                kind,
                members,
                ..
            } => {
                if *kind == ClassKind::Struct {
                    self.check_struct_defaults(members);
                }
            }
            Stmt::TypeAlias { .. } => {}
            _ => {}
        }
    }

    fn check_expr(
        &mut self,
        expr: ExprId<'a>,
        env: &mut HashMap<String, Type>,
        explicit: &mut HashSet<String>,
    ) -> Type {
        match *expr {
            Expr::Null { .. } => Type::Primitive(PrimitiveType::Null),
            Expr::Assign { var, expr, .. } | Expr::AssignRef { var, expr, .. } => {
                let rhs_ty = self.check_expr(expr, env, explicit);
                self.assign_to_target(var, &rhs_ty, env, explicit);
                rhs_ty
            }
            Expr::AssignOp { var, expr, .. } => {
                let rhs_ty = self.check_expr(expr, env, explicit);
                self.assign_to_target(var, &rhs_ty, env, explicit);
                rhs_ty
            }
            Expr::DotAccess {
                target,
                property,
                span,
            } => {
                self.check_dot_access(target, property, span, env);
                self.infer_expr_with_env(expr, env)
            }
            Expr::Binary {
                left,
                right,
                op,
                span,
            } => {
                if self.is_null_comparison(op, left, right) && !self.allow_null_comparisons() {
                    self.errors.push(TypeError {
                        span,
                        message:
                            "Null comparisons are not allowed in PHPX; use isset() instead"
                                .to_string(),
                    });
                }
                let _ = self.check_expr(left, env, explicit);
                let _ = self.check_expr(right, env, explicit);
                self.infer_expr_with_env(expr, env)
            }
            Expr::Unary { expr, .. } => {
                let _ = self.check_expr(expr, env, explicit);
                self.infer_expr_with_env(expr, env)
            }
            Expr::Call { func, args, .. } => {
                let _ = self.check_expr(func, env, explicit);
                for arg in args.iter() {
                    let _ = self.check_expr(arg.value, env, explicit);
                }
                self.check_call_signature(func, args, env)
            }
            Expr::New { class, args, span } => {
                let _ = self.check_expr(class, env, explicit);
                for arg in args.iter() {
                    let _ = self.check_expr(arg.value, env, explicit);
                }
                self.errors.push(TypeError {
                    span,
                    message: "new is not allowed in PHPX; use struct literals".to_string(),
                });
                Type::Unknown
            }
            Expr::MethodCall {
                target,
                method,
                args,
                span,
            } => {
                let target_ty = self.check_expr(target, env, explicit);
                for arg in args.iter() {
                    let _ = self.check_expr(arg.value, env, explicit);
                }
                self.check_method_call_signature(&target_ty, method, args, env, span)
            }
            Expr::NullsafeMethodCall {
                target,
                method,
                args,
                span,
            } => {
                let target_ty = self.check_expr(target, env, explicit);
                for arg in args.iter() {
                    let _ = self.check_expr(arg.value, env, explicit);
                }
                self.check_method_call_signature(&target_ty, method, args, env, span)
            }
            Expr::StaticCall {
                class,
                method,
                args,
                span,
            } => {
                let _ = self.check_expr(class, env, explicit);
                for arg in args.iter() {
                    let _ = self.check_expr(arg.value, env, explicit);
                }
                if let Some((enum_name, case_name, case_info)) =
                    self.enum_case_lookup(class, method)
                {
                    self.check_enum_case_call(
                        &enum_name,
                        &case_name,
                        &case_info,
                        args,
                        span,
                        env,
                    );
                    if enum_name.eq_ignore_ascii_case("Option") {
                        let arg_ty = args
                            .get(0)
                            .map(|arg| self.infer_expr_with_env(arg.value, env))
                            .unwrap_or(Type::Unknown);
                        let case_args = if case_name.eq_ignore_ascii_case("Some") {
                            vec![arg_ty]
                        } else {
                            Vec::new()
                        };
                        return Type::EnumCase {
                            enum_name: "Option".to_string(),
                            case_name,
                            args: case_args,
                        };
                    }
                    if enum_name.eq_ignore_ascii_case("Result") {
                        let arg_ty = args
                            .get(0)
                            .map(|arg| self.infer_expr_with_env(arg.value, env))
                            .unwrap_or(Type::Unknown);
                        let case_args = if case_name.eq_ignore_ascii_case("Ok") {
                            vec![arg_ty, Type::Unknown]
                        } else if case_name.eq_ignore_ascii_case("Err") {
                            vec![Type::Unknown, arg_ty]
                        } else {
                            Vec::new()
                        };
                        return Type::EnumCase {
                            enum_name: "Result".to_string(),
                            case_name,
                            args: case_args,
                        };
                    }
                    return Type::EnumCase {
                        enum_name,
                        case_name,
                        args: Vec::new(),
                    };
                }
                self.check_static_class_ref(class, span);
                Type::Unknown
            }
            Expr::ClassConstFetch {
                class,
                constant,
                span,
            } => {
                let _ = self.check_expr(class, env, explicit);
                if let Some((enum_name, case_name, _)) = self.enum_case_lookup(class, constant) {
                    if enum_name.eq_ignore_ascii_case("Option")
                        || enum_name.eq_ignore_ascii_case("Result")
                    {
                        return Type::EnumCase {
                            enum_name: if enum_name.eq_ignore_ascii_case("Option") {
                                "Option".to_string()
                            } else {
                                "Result".to_string()
                            },
                            case_name,
                            args: Vec::new(),
                        };
                    }
                    return Type::Enum(enum_name);
                }
                self.check_static_class_ref(class, span);
                Type::Unknown
            }
            Expr::PropertyFetch { target, .. }
            | Expr::NullsafePropertyFetch { target, .. } => {
                let _ = self.check_expr(target, env, explicit);
                Type::Unknown
            }
            Expr::Array { items, .. } => {
                for item in items.iter() {
                    if let Some(key) = item.key {
                        let _ = self.check_expr(key, env, explicit);
                    }
                    let _ = self.check_expr(item.value, env, explicit);
                }
                Type::Array
            }
            Expr::ObjectLiteral { items, .. } => {
                for item in items.iter() {
                    let _ = self.check_expr(item.value, env, explicit);
                }
                self.infer_expr_with_env(expr, env)
            }
            Expr::JsxElement {
                attributes,
                children,
                ..
            } => {
                for attr in attributes.iter() {
                    if let Some(value) = attr.value {
                        self.validate_jsx_expr(value);
                        let _ = self.check_expr(value, env, explicit);
                    }
                }
                for child in children.iter() {
                    if let JsxChild::Expr(expr) = *child {
                        self.validate_jsx_expr(expr);
                        let _ = self.check_expr(expr, env, explicit);
                    }
                }
                Type::VNode
            }
            Expr::JsxFragment { children, .. } => {
                for child in children.iter() {
                    if let JsxChild::Expr(expr) = *child {
                        self.validate_jsx_expr(expr);
                        let _ = self.check_expr(expr, env, explicit);
                    }
                }
                Type::VNode
            }
            Expr::StructLiteral { name, fields, span } => {
                let raw = token_text(self.source, name.span);
                let struct_name = raw.trim_start_matches('\\').to_string();
                let info = if let Some(info) = self.structs.get(&struct_name) {
                    info.clone()
                } else {
                    self.errors.push(TypeError {
                        span: span,
                        message: format!("Unknown struct '{}'", struct_name),
                    });
                    return Type::Unknown;
                };

                let mut seen = HashSet::new();
                for field in fields.iter() {
                    let field_name = token_text(self.source, field.name.span);
                    let field_name = field_name.trim_start_matches('$').to_string();

                    if !seen.insert(field_name.clone()) {
                        self.errors.push(TypeError {
                            span: field.span,
                            message: format!(
                                "Duplicate field '{}' in struct literal '{}'",
                                field_name, struct_name
                            ),
                        });
                        continue;
                    }

                    let expected = info.fields.get(&field_name);
                    if expected.is_none() {
                        self.errors.push(TypeError {
                            span: field.span,
                            message: format!(
                                "Unknown field '{}' in struct literal '{}'",
                                field_name, struct_name
                            ),
                        });
                    }

                    let actual = self.check_expr(field.value, env, explicit);
                    if let Some(expected) = expected {
                        if !self.is_assignable(&actual, expected) {
                            self.errors.push(TypeError {
                                span: field.span,
                                message: format!(
                                    "Field '{}' expects {}, got {}",
                                    field_name, expected, actual
                                ),
                            });
                        }
                    }
                }

                for field in info.fields.keys() {
                    if info.defaults.contains(field) {
                        continue;
                    }
                    if !seen.contains(field) {
                        self.errors.push(TypeError {
                            span: span,
                            message: format!(
                                "Missing field '{}' in struct literal '{}'",
                                field, struct_name
                            ),
                        });
                    }
                }

                Type::Struct(struct_name)
            }
            Expr::ArrayDimFetch { array, dim, .. } => {
                let _ = self.check_expr(array, env, explicit);
                if let Some(dim) = dim {
                    let _ = self.check_expr(dim, env, explicit);
                }
                Type::Unknown
            }
            Expr::Ternary {
                condition,
                if_true,
                if_false,
                ..
            } => {
                let _ = self.check_expr(condition, env, explicit);
                if let Some(if_true) = if_true {
                    let _ = self.check_expr(if_true, env, explicit);
                }
                let _ = self.check_expr(if_false, env, explicit);
                Type::Unknown
            }
            Expr::Match { condition, arms, .. } => {
                let cond_ty = self.check_expr(condition, env, explicit);
                let mut match_ty = Type::Unknown;
                for arm in arms.iter() {
                    let mut arm_env = env.clone();
                    let mut arm_explicit = explicit.clone();
                    self.apply_match_arm_narrowing(condition, arm, &mut arm_env);
                    if let Some(conds) = arm.conditions {
                        for cond in conds.iter() {
                            let _ = self.check_expr(cond, &mut arm_env, &mut arm_explicit);
                        }
                    }
                    let body_ty = self.check_expr(arm.body, &mut arm_env, &mut arm_explicit);
                    match_ty = merge_types(&match_ty, &body_ty);
                }
                self.check_match_exhaustive(&cond_ty, arms, env);
                match_ty
            }
            Expr::AnonymousClass { span, .. } => {
                self.errors.push(TypeError {
                    span,
                    message: "Anonymous classes are not allowed in PHPX".to_string(),
                });
                Type::Unknown
            }
            Expr::Closure { body, .. } => {
                let mut inner_env = env.clone();
                let mut inner_explicit = explicit.clone();
                for stmt in body.iter() {
                    self.check_stmt(stmt, &mut inner_env, &mut inner_explicit, None);
                }
                Type::Unknown
            }
            Expr::ArrowFunction { expr, .. } => {
                let _ = self.check_expr(expr, env, explicit);
                Type::Unknown
            }
            Expr::Include { expr, .. }
            | Expr::Print { expr, .. }
            | Expr::Clone { expr, .. }
            | Expr::Cast { expr, .. }
            | Expr::Empty { expr, .. }
            | Expr::Eval { expr, .. } => {
                let _ = self.check_expr(expr, env, explicit);
                Type::Unknown
            }
            Expr::Isset { vars, .. } => {
                for var in vars.iter() {
                    let _ = self.check_expr(var, env, explicit);
                }
                Type::Unknown
            }
            Expr::Yield { key, value, .. } => {
                if let Some(key) = key {
                    let _ = self.check_expr(key, env, explicit);
                }
                if let Some(value) = value {
                    let _ = self.check_expr(value, env, explicit);
                }
                Type::Unknown
            }
            Expr::Die { expr, .. } | Expr::Exit { expr, .. } => {
                if let Some(expr) = expr {
                    let _ = self.check_expr(expr, env, explicit);
                }
                Type::Unknown
            }
            Expr::PostInc { var, .. } | Expr::PostDec { var, .. } => {
                let _ = self.check_expr(var, env, explicit);
                Type::Unknown
            }
            _ => self.infer_expr_with_env(expr, env),
        }
    }

    fn infer_expr_with_env(&self, expr: ExprId<'a>, env: &HashMap<String, Type>) -> Type {
        let ctx = InferContext {
            source: self.source,
            vars: env,
            structs: &self.structs,
            functions: &self.function_returns,
            enums: &self.enums,
        };
        infer_expr(expr, &ctx)
    }

    fn extract_static_ident(&self, expr: ExprId<'a>) -> Option<String> {
        match *expr {
            Expr::Variable { span, .. } => {
                let name = token_text(self.source, span);
                if name.starts_with('$') {
                    None
                } else {
                    Some(name)
                }
            }
            _ => None,
        }
    }

    fn enum_case_lookup(
        &self,
        class: ExprId<'a>,
        member: ExprId<'a>,
    ) -> Option<(String, String, EnumCaseInfo)> {
        let class_name = self.extract_static_ident(class)?;
        let case_name = self.extract_static_ident(member)?;
        if let Some(case_info) = self.builtin_enum_case_info(&class_name, &case_name) {
            let enum_name = if class_name.eq_ignore_ascii_case("Option") {
                "Option".to_string()
            } else {
                "Result".to_string()
            };
            let canonical_case = if enum_name == "Option" {
                if case_name.eq_ignore_ascii_case("Some") {
                    "Some"
                } else {
                    "None"
                }
            } else if case_name.eq_ignore_ascii_case("Ok") {
                "Ok"
            } else {
                "Err"
            };
            return Some((enum_name, canonical_case.to_string(), case_info));
        }
        let info = self.enums.get(&class_name)?;
        let case_info = info.cases.get(&case_name)?;
        Some((class_name, case_name, case_info.clone()))
    }

    fn enum_case_from_expr(&self, expr: ExprId<'a>) -> Option<(String, String)> {
        match *expr {
            Expr::ClassConstFetch { class, constant, .. } => {
                self.enum_case_lookup(class, constant)
                    .map(|(enum_name, case_name, _)| (enum_name, case_name))
            }
            Expr::StaticCall { class, method, .. } => self
                .enum_case_lookup(class, method)
                .map(|(enum_name, case_name, _)| (enum_name, case_name)),
            _ => None,
        }
    }

    fn builtin_enum_case_info(&self, enum_name: &str, case_name: &str) -> Option<EnumCaseInfo> {
        if enum_name.eq_ignore_ascii_case("Option") {
            if case_name.eq_ignore_ascii_case("Some") {
                return Some(EnumCaseInfo {
                    params: vec![EnumParamInfo {
                        name: "value".to_string(),
                        ty: None,
                        required: true,
                        variadic: false,
                    }],
                });
            }
            if case_name.eq_ignore_ascii_case("None") {
                return Some(EnumCaseInfo { params: Vec::new() });
            }
        }
        if enum_name.eq_ignore_ascii_case("Result") {
            if case_name.eq_ignore_ascii_case("Ok") {
                return Some(EnumCaseInfo {
                    params: vec![EnumParamInfo {
                        name: "value".to_string(),
                        ty: None,
                        required: true,
                        variadic: false,
                    }],
                });
            }
            if case_name.eq_ignore_ascii_case("Err") {
                return Some(EnumCaseInfo {
                    params: vec![EnumParamInfo {
                        name: "error".to_string(),
                        ty: None,
                        required: true,
                        variadic: false,
                    }],
                });
            }
        }
        None
    }

    fn builtin_enum_cases(&self, enum_name: &str) -> Option<Vec<String>> {
        if enum_name.eq_ignore_ascii_case("Option") {
            return Some(vec!["Some".to_string(), "None".to_string()]);
        }
        if enum_name.eq_ignore_ascii_case("Result") {
            return Some(vec!["Ok".to_string(), "Err".to_string()]);
        }
        None
    }

    fn builtin_enum_args_from_type(&self, enum_name: &str, ty: &Type) -> Option<Vec<Type>> {
        match ty {
            Type::Applied { base, args } if base.eq_ignore_ascii_case(enum_name) => {
                Some(args.clone())
            }
            Type::EnumCase {
                enum_name: name,
                args,
                ..
            } if name.eq_ignore_ascii_case(enum_name) && !args.is_empty() => Some(args.clone()),
            Type::Union(types) => {
                for ty in types {
                    if let Some(args) = self.builtin_enum_args_from_type(enum_name, ty) {
                        return Some(args);
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn check_enum_case_call(
        &mut self,
        enum_name: &str,
        case_name: &str,
        case_info: &EnumCaseInfo,
        args: &'a [crate::parser::ast::Arg<'a>],
        span: Span,
        env: &HashMap<String, Type>,
    ) {
        if case_info.params.is_empty() {
            self.errors.push(TypeError {
                span,
                message: format!(
                    "Enum case {}::{} has no payload; use {}::{} without calling it",
                    enum_name, case_name, enum_name, case_name
                ),
            });
            return;
        }

        if args.len() != case_info.params.len() {
            self.errors.push(TypeError {
                span,
                message: format!(
                    "Enum case {}::{} expects {} arguments, got {}",
                    enum_name,
                    case_name,
                    case_info.params.len(),
                    args.len()
                ),
            });
            return;
        }

        for (idx, param) in case_info.params.iter().enumerate() {
            let arg = &args[idx];
            let actual = self.infer_expr_with_env(arg.value, env);
            if let Some(expected) = &param.ty {
                if let Expr::ObjectLiteral { items, span: obj_span } = *arg.value {
                    self.check_object_literal_against_type(items, expected, obj_span, env);
                }
                if !self.is_assignable(&actual, expected) {
                    self.errors.push(TypeError {
                        span: arg.span,
                        message: format!(
                            "Enum case {}::{} argument {} has type {}, expected {}",
                            enum_name,
                            case_name,
                            idx + 1,
                            actual,
                            expected
                        ),
                    });
                }
            }
        }
    }

    fn enum_names_from_type(&self, ty: &Type) -> Option<(Vec<String>, bool)> {
        match ty {
            Type::Enum(name) => Some((vec![name.clone()], false)),
            Type::EnumCase { enum_name, .. } => Some((vec![enum_name.clone()], false)),
            Type::Applied { base, .. }
                if base.eq_ignore_ascii_case("Option")
                    || base.eq_ignore_ascii_case("Result") =>
            {
                let name = if base.eq_ignore_ascii_case("Option") {
                    "Option".to_string()
                } else {
                    "Result".to_string()
                };
                Some((vec![name], false))
            }
            Type::Union(types) => {
                let mut names = Vec::new();
                let mut has_null = false;
                for ty in types.iter() {
                    match ty {
                        Type::Enum(name) => names.push(name.clone()),
                        Type::EnumCase { enum_name, .. } => names.push(enum_name.clone()),
                        Type::Applied { base, .. }
                            if base.eq_ignore_ascii_case("Option")
                                || base.eq_ignore_ascii_case("Result") =>
                        {
                            let name = if base.eq_ignore_ascii_case("Option") {
                                "Option".to_string()
                            } else {
                                "Result".to_string()
                            };
                            names.push(name);
                        }
                        Type::Primitive(PrimitiveType::Null) => has_null = true,
                        _ => return None,
                    }
                }
                if names.is_empty() {
                    None
                } else {
                    Some((names, has_null))
                }
            }
            _ => None,
        }
    }

    fn check_match_exhaustive(
        &mut self,
        cond_ty: &Type,
        arms: &'a [crate::parser::ast::MatchArm<'a>],
        _env: &HashMap<String, Type>,
    ) {
        let Some((enum_names, allows_null)) = self.enum_names_from_type(cond_ty) else {
            return;
        };

        let mut covered: HashMap<String, HashSet<String>> = HashMap::new();
        for name in enum_names.iter() {
            covered.insert(name.clone(), HashSet::new());
        }
        let mut null_covered = false;

        for arm in arms.iter() {
            let Some(conds) = arm.conditions else {
                return;
            };
            for cond in conds.iter() {
                if matches!(*cond, Expr::Null { .. }) {
                    null_covered = true;
                    continue;
                }
                if let Some((enum_name, case_name)) = self.enum_case_from_expr(*cond) {
                    if let Some(entry) = covered.get_mut(&enum_name) {
                        entry.insert(case_name);
                    } else {
                        self.errors.push(TypeError {
                            span: arm.span,
                            message: format!(
                                "Match arm uses enum case '{}::{}' that is not part of this match",
                                enum_name, case_name
                            ),
                        });
                        return;
                    }
                } else {
                    // Mixed conditions: skip exhaustiveness checking.
                    return;
                }
            }
        }

        for enum_name in enum_names.iter() {
            let case_names = if let Some(info) = self.enums.get(enum_name) {
                info.cases.keys().cloned().collect::<Vec<_>>()
            } else if let Some(builtin) = self.builtin_enum_cases(enum_name) {
                builtin
            } else {
                continue;
            };
            let Some(seen) = covered.get(enum_name) else {
                continue;
            };
            for case_name in case_names.iter() {
                if !seen.contains(case_name) {
                    self.errors.push(TypeError {
                        span: arms.last().map(|arm| arm.span).unwrap_or_default(),
                        message: format!(
                            "Match on {} is not exhaustive; missing case {}::{}",
                            enum_name, enum_name, case_name
                        ),
                    });
                    return;
                }
            }
        }

        if allows_null && !null_covered {
            self.errors.push(TypeError {
                span: arms.last().map(|arm| arm.span).unwrap_or_default(),
                message: "Match on nullable enum is not exhaustive; missing null arm".to_string(),
            });
        }
    }

    fn apply_match_arm_narrowing(
        &self,
        condition: ExprId<'a>,
        arm: &crate::parser::ast::MatchArm<'a>,
        env: &mut HashMap<String, Type>,
    ) {
        let Some(var_name) = self.extract_var_name(condition) else {
            return;
        };
        let Some(conds) = arm.conditions else {
            return;
        };
        let current_ty = env.get(&var_name);
        let mut cases = Vec::new();
        for cond in conds.iter() {
            let Some((enum_name, case_name)) = self.enum_case_from_expr(*cond) else {
                return;
            };
            cases.push((enum_name, case_name));
        }
        if cases.is_empty() {
            return;
        }
        let narrowed = if cases.len() == 1 {
            let (enum_name, case_name) = &cases[0];
            let args = current_ty
                .and_then(|ty| self.builtin_enum_args_from_type(enum_name, ty))
                .unwrap_or_default();
            Type::EnumCase {
                enum_name: enum_name.clone(),
                case_name: case_name.clone(),
                args,
            }
        } else {
            let mut variants = Vec::new();
            for (enum_name, case_name) in cases.into_iter() {
                let args = current_ty
                    .and_then(|ty| self.builtin_enum_args_from_type(&enum_name, ty))
                    .unwrap_or_default();
                variants.push(Type::EnumCase {
                    enum_name,
                    case_name,
                    args,
                });
            }
            Type::Union(variants)
        };
        env.insert(var_name, narrowed);
    }

    fn narrow_env_for_condition(
        &self,
        condition: ExprId<'a>,
        env: &HashMap<String, Type>,
        truthy: bool,
    ) -> HashMap<String, Type> {
        let mut out = env.clone();
        self.apply_condition_narrowing(condition, &mut out, truthy);
        out
    }

    fn apply_condition_narrowing(
        &self,
        condition: ExprId<'a>,
        env: &mut HashMap<String, Type>,
        truthy: bool,
    ) {
        match *condition {
            Expr::Binary { op, left, right, .. } => {
                if let Some(var_name) = self.null_compare_var(left, right) {
                    match op {
                        BinaryOp::EqEqEq | BinaryOp::EqEq => {
                            if truthy {
                                self.narrow_var_to_null(&var_name, env);
                            } else {
                                self.remove_null_from_var(&var_name, env);
                            }
                        }
                        BinaryOp::NotEqEq | BinaryOp::NotEq => {
                            if truthy {
                                self.remove_null_from_var(&var_name, env);
                            } else {
                                self.narrow_var_to_null(&var_name, env);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Expr::Isset { vars, .. } => {
                if truthy {
                    for var in vars.iter() {
                        if let Some(name) = self.extract_var_name(*var) {
                            self.remove_null_from_var(&name, env);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn null_compare_var(&self, left: ExprId<'a>, right: ExprId<'a>) -> Option<String> {
        if matches!(*left, Expr::Null { .. }) {
            return self.extract_var_name(right);
        }
        if matches!(*right, Expr::Null { .. }) {
            return self.extract_var_name(left);
        }
        None
    }

    fn extract_var_name(&self, expr: ExprId<'a>) -> Option<String> {
        match *expr {
            Expr::Variable { span, .. } => {
                let name = token_text(self.source, span);
                Some(name.trim_start_matches('$').to_string())
            }
            _ => None,
        }
    }

    fn check_static_class_ref(&mut self, class: ExprId<'a>, span: Span) {
        let Some(name) = self.extract_static_ident(class) else {
            self.errors.push(TypeError {
                span,
                message: "Dynamic class references are not allowed in PHPX".to_string(),
            });
            return;
        };
        if self.structs.contains_key(&name) || self.enums.contains_key(&name) {
            return;
        }
        self.errors.push(TypeError {
            span,
            message: format!(
                "Unknown type '{}' in PHPX; classes are not allowed",
                name
            ),
        });
    }

    fn struct_name_from_expr(&self, expr: ExprId<'a>) -> Option<String> {
        match *expr {
            Expr::Variable { span, .. } => {
                let name = token_text(self.source, span);
                if name.starts_with('$') {
                    None
                } else {
                    Some(name)
                }
            }
            _ => None,
        }
    }

    fn is_struct_new_target(&self, expr: ExprId<'a>) -> bool {
        let Some(name) = self.struct_name_from_expr(expr) else {
            return false;
        };
        self.structs.contains_key(&name)
    }

    fn remove_null_from_var(&self, name: &str, env: &mut HashMap<String, Type>) {
        let Some(existing) = env.get(name) else {
            return;
        };
        let updated = remove_null(existing);
        env.insert(name.to_string(), updated);
    }

    fn narrow_var_to_null(&self, name: &str, env: &mut HashMap<String, Type>) {
        let Some(existing) = env.get(name) else {
            return;
        };
        let updated = keep_only_null(existing);
        env.insert(name.to_string(), updated);
    }

    fn is_null_comparison(&self, op: BinaryOp, left: ExprId<'a>, right: ExprId<'a>) -> bool {
        matches!(
            op,
            BinaryOp::EqEqEq | BinaryOp::EqEq | BinaryOp::NotEqEq | BinaryOp::NotEq | BinaryOp::Spaceship
        ) && (matches!(*left, Expr::Null { .. }) || matches!(*right, Expr::Null { .. }))
    }

    fn allow_null_comparisons(&self) -> bool {
        let Some(path) = self.file_path.as_ref() else {
            return false;
        };
        let Some(root) = find_modules_root(path) else {
            return false;
        };
        root.join("stdlib.json").is_file()
    }

    fn check_dot_access(
        &mut self,
        target: ExprId<'a>,
        property: &'a crate::parser::lexer::token::Token,
        span: Span,
        env: &HashMap<String, Type>,
    ) {
        let target_ty = self.infer_expr_with_env(target, env);
        let prop_name = token_text(self.source, property.span);
        match target_ty {
            Type::ObjectShape(fields) => {
                if !fields.contains_key(&prop_name) {
                    self.errors.push(TypeError {
                        span,
                        message: format!(
                            "Unknown object field '{}'",
                            prop_name
                        ),
                    });
                }
            }
            Type::Struct(name) => {
                if !self.structs.contains_key(&name) {
                    return;
                }
                match self.resolve_struct_field(&name, &prop_name) {
                    StructFieldResolution::Found(_) => {}
                    StructFieldResolution::Ambiguous => {
                        self.errors.push(TypeError {
                            span,
                            message: format!(
                                "Ambiguous promoted field '{}::{}'",
                                name, prop_name
                            ),
                        });
                    }
                    StructFieldResolution::Missing => {
                        self.errors.push(TypeError {
                            span,
                            message: format!(
                                "Unknown struct field '{}::{}'",
                                name, prop_name
                            ),
                        });
                    }
                }
            }
            Type::Enum(name) => {
                if !self.enum_allows_field(&name, &prop_name) {
                    self.errors.push(TypeError {
                        span,
                        message: format!(
                            "Unknown enum field '{}::{}'",
                            name, prop_name
                        ),
                    });
                }
            }
            Type::EnumCase {
                enum_name,
                case_name,
                ..
            } => {
                if !self.enum_case_allows_field(&enum_name, &case_name, &prop_name) {
                    self.errors.push(TypeError {
                        span,
                        message: format!(
                            "Unknown enum field '{}::{}::{}'",
                            enum_name, case_name, prop_name
                        ),
                    });
                }
            }
            Type::Applied { base, .. }
                if base.eq_ignore_ascii_case("Option")
                    || base.eq_ignore_ascii_case("Result") =>
            {
                if prop_name != "name" {
                    self.errors.push(TypeError {
                        span,
                        message: format!(
                            "Unknown enum field '{}::{}'",
                            base, prop_name
                        ),
                    });
                }
            }
            Type::Union(types) => {
                let mut invalid = false;
                let mut missing = false;
                let mut any_ok = false;
                for ty in types.iter() {
                    match ty {
                        Type::Primitive(PrimitiveType::Null) => {
                            any_ok = true;
                        }
                        Type::ObjectShape(fields) => {
                            any_ok = true;
                            if !fields.contains_key(&prop_name) {
                                missing = true;
                            }
                        }
                        Type::Struct(name) => {
                            any_ok = true;
                            if !self.structs.contains_key(name) {
                                continue;
                            }
                            match self.resolve_struct_field(name, &prop_name) {
                                StructFieldResolution::Found(_) => {}
                                StructFieldResolution::Missing => {
                                    missing = true;
                                }
                                StructFieldResolution::Ambiguous => {
                                    invalid = true;
                                }
                            }
                        }
                        Type::Enum(name) => {
                            any_ok = true;
                            if !self.enum_allows_field(name, &prop_name) {
                                missing = true;
                            }
                        }
                        Type::Applied { base, .. }
                            if base.eq_ignore_ascii_case("Option")
                                || base.eq_ignore_ascii_case("Result") =>
                        {
                            any_ok = true;
                            if prop_name != "name" {
                                missing = true;
                            }
                        }
                        Type::EnumCase {
                            enum_name,
                            case_name,
                            ..
                        } => {
                            any_ok = true;
                            if !self.enum_case_allows_field(enum_name, case_name, &prop_name) {
                                missing = true;
                            }
                        }
                        Type::Object | Type::Mixed | Type::Unknown => {
                            any_ok = true;
                        }
                        _ => {
                            invalid = true;
                        }
                    }
                }
                if invalid || (any_ok && missing) {
                    self.errors.push(TypeError {
                        span,
                        message: format!(
                            "Unknown object field '{}' for union type",
                            prop_name
                        ),
                    });
                }
            }
            _ => {}
        }
    }

    fn resolve_struct_field(&self, struct_name: &str, field: &str) -> StructFieldResolution {
        let mut visited = HashSet::new();
        self.resolve_struct_field_inner(struct_name, field, &mut visited)
    }

    fn resolve_struct_field_inner(
        &self,
        struct_name: &str,
        field: &str,
        visited: &mut HashSet<String>,
    ) -> StructFieldResolution {
        if !visited.insert(struct_name.to_string()) {
            return StructFieldResolution::Missing;
        }
        let Some(info) = self.structs.get(struct_name) else {
            return StructFieldResolution::Missing;
        };

        if let Some(ty) = info.fields.get(field) {
            return StructFieldResolution::Found(ty.clone());
        }

        let mut found: Option<Type> = None;
        let mut ambiguous = false;

        for embed in &info.embeds {
            match self.resolve_struct_field_inner(embed, field, visited) {
                StructFieldResolution::Found(ty) => {
                    if found.is_some() {
                        ambiguous = true;
                    } else {
                        found = Some(ty);
                    }
                }
                StructFieldResolution::Ambiguous => {
                    ambiguous = true;
                }
                StructFieldResolution::Missing => {}
            }
        }

        if ambiguous {
            StructFieldResolution::Ambiguous
        } else if let Some(ty) = found {
            StructFieldResolution::Found(ty)
        } else {
            StructFieldResolution::Missing
        }
    }

    fn enum_allows_field(&self, enum_name: &str, field: &str) -> bool {
        if enum_name.eq_ignore_ascii_case("Option") || enum_name.eq_ignore_ascii_case("Result") {
            return field == "name";
        }
        let Some(info) = self.enums.get(enum_name) else {
            return false;
        };
        if field == "name" {
            return true;
        }
        if field == "value" {
            return info.backed.is_some();
        }
        for case in info.cases.values() {
            if !case.params.iter().any(|param| param.name == field) {
                return false;
            }
        }
        !info.cases.is_empty()
    }

    fn enum_case_allows_field(&self, enum_name: &str, case_name: &str, field: &str) -> bool {
        if enum_name.eq_ignore_ascii_case("Option") {
            if field == "name" {
                return true;
            }
            return case_name.eq_ignore_ascii_case("Some") && field == "value";
        }
        if enum_name.eq_ignore_ascii_case("Result") {
            if field == "name" {
                return true;
            }
            if case_name.eq_ignore_ascii_case("Ok") {
                return field == "value";
            }
            if case_name.eq_ignore_ascii_case("Err") {
                return field == "error";
            }
            return false;
        }
        let Some(info) = self.enums.get(enum_name) else {
            return false;
        };
        if field == "name" {
            return true;
        }
        if field == "value" {
            return info.backed.is_some();
        }
        let Some(case) = info.cases.get(case_name) else {
            return false;
        };
        case.params.iter().any(|param| param.name == field)
    }

    fn type_allows_null(&self, ty: &Type) -> bool {
        if matches!(ty, Type::Primitive(PrimitiveType::Null)) {
            return true;
        }
        match ty {
            Type::Union(types) => types.iter().any(|t| self.type_allows_null(t)),
            _ => false,
        }
    }

    fn check_object_literal_against_type(
        &mut self,
        items: &'a [crate::parser::ast::ObjectItem<'a>],
        expected: &Type,
        span: Span,
        env: &HashMap<String, Type>,
    ) {
        let Type::ObjectShape(expected_fields) = expected else {
            return;
        };
        self.check_object_literal_against_shape(items, expected_fields, span, env);
    }

    fn check_object_literal_against_shape(
        &mut self,
        items: &'a [crate::parser::ast::ObjectItem<'a>],
        expected: &BTreeMap<String, ObjectField>,
        span: Span,
        env: &HashMap<String, Type>,
    ) {
        let mut seen = HashSet::new();
        for item in items.iter() {
            let key = object_key_name(item.key, self.source);
            seen.insert(key.clone());
            let Some(expected_field) = expected.get(&key) else {
                self.errors.push(TypeError {
                    span: item.span,
                    message: format!("Unknown object field '{}' in object literal", key),
                });
                continue;
            };
            let actual = self.infer_expr_with_env(item.value, env);
            if !self.is_assignable(&actual, &expected_field.ty) {
                self.errors.push(TypeError {
                    span: item.span,
                    message: format!(
                        "Object field '{}' has type {}, expected {}",
                        key,
                        actual,
                        expected_field.ty
                    ),
                });
            }
        }

        for (name, field) in expected.iter() {
            if field.optional {
                continue;
            }
            if !seen.contains(name) {
                self.errors.push(TypeError {
                    span,
                    message: format!("Missing required object field '{}'", name),
                });
            }
        }
    }

    fn infer_type_params(
        &mut self,
        pattern: &Type,
        actual: &Type,
        inferred: &mut HashMap<String, Type>,
    ) {
        match pattern {
            Type::TypeParam(name) => {
                if let Some(existing) = inferred.get(name) {
                    if matches!(existing, Type::Unknown) {
                        inferred.insert(name.clone(), actual.clone());
                    } else if !self.is_assignable(actual, existing) {
                        let merged = merge_types(existing, actual);
                        inferred.insert(name.clone(), merged);
                    }
                } else {
                    inferred.insert(name.clone(), actual.clone());
                }
            }
            Type::ObjectShape(fields) => {
                if let Type::ObjectShape(actual_fields) = actual {
                    for (name, field) in fields.iter() {
                        if let Some(actual_field) = actual_fields.get(name) {
                            self.infer_type_params(&field.ty, &actual_field.ty, inferred);
                        }
                    }
                }
            }
            Type::Union(options) => {
                for opt in options.iter() {
                    if self.is_assignable(actual, opt) {
                        self.infer_type_params(opt, actual, inferred);
                        break;
                    }
                }
            }
            Type::Applied { base, args } => {
                if let Type::Applied {
                    base: actual_base,
                    args: actual_args,
                } = actual
                {
                    if base == actual_base && args.len() == actual_args.len() {
                        for (idx, arg) in args.iter().enumerate() {
                            self.infer_type_params(arg, &actual_args[idx], inferred);
                        }
                    }
                } else if let Type::EnumCase {
                    enum_name,
                    case_name,
                    args: actual_args,
                } = actual
                {
                    if base.eq_ignore_ascii_case(enum_name) {
                        if base.eq_ignore_ascii_case("Option") && args.len() == 1 {
                            if case_name.eq_ignore_ascii_case("Some") {
                                if let Some(actual_inner) = actual_args.get(0) {
                                    self.infer_type_params(&args[0], actual_inner, inferred);
                                }
                            }
                        }
                        if base.eq_ignore_ascii_case("Result") && args.len() == 2 {
                            if case_name.eq_ignore_ascii_case("Ok") {
                                if let Some(actual_ok) = actual_args.get(0) {
                                    self.infer_type_params(&args[0], actual_ok, inferred);
                                }
                            } else if case_name.eq_ignore_ascii_case("Err") {
                                if let Some(actual_err) = actual_args.get(1) {
                                    self.infer_type_params(&args[1], actual_err, inferred);
                                }
                            }
                        }
                    }
                } else if base.eq_ignore_ascii_case("array")
                    && matches!(actual, Type::Array)
                    && args.len() == 1
                {
                    self.infer_type_params(&args[0], &Type::Unknown, inferred);
                }
            }
            _ => {}
        }
    }

    fn assign_to_target(
        &mut self,
        target: ExprId<'a>,
        value_ty: &Type,
        env: &mut HashMap<String, Type>,
        explicit: &mut HashSet<String>,
    ) {
        match *target {
            Expr::Variable { span, .. } => {
                let name = token_text(self.source, span);
                let name = name.trim_start_matches('$').to_string();
                let is_null = matches!(value_ty, Type::Primitive(PrimitiveType::Null));
                if let Some(existing) = env.get(&name) {
                    if explicit.contains(&name) {
                        if is_null && !self.type_allows_null(existing) {
                            self.errors.push(TypeError {
                                span,
                                message: "Null is not allowed in PHPX; use Option<T> instead"
                                    .to_string(),
                            });
                        }
                        if !self.is_assignable(value_ty, existing) {
                            self.errors.push(TypeError {
                                span,
                                message: format!(
                                    "Type mismatch: expected {}, got {}",
                                    existing, value_ty
                                ),
                            });
                        }
                    } else {
                        if is_null {
                            self.errors.push(TypeError {
                                span,
                                message: "Null is not allowed in PHPX; use Option<T> instead"
                                    .to_string(),
                            });
                        }
                        let merged = merge_types(existing, value_ty);
                        env.insert(name.clone(), merged);
                    }
                } else {
                    if is_null {
                        self.errors.push(TypeError {
                            span,
                            message: "Null is not allowed in PHPX; use Option<T> instead"
                                .to_string(),
                        });
                    }
                    env.insert(name.clone(), value_ty.clone());
                }
            }
            Expr::DotAccess {
                target,
                property,
                span,
            } => {
                self.check_dot_access(target, property, span, env);
            }
            _ => {}
        }
    }

    fn collect_struct_names(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            if let Stmt::Class { kind, name, .. } = stmt {
                if *kind != ClassKind::Struct {
                    continue;
                }
                let class_name = token_text(self.source, name.span);
                self.structs
                    .entry(class_name)
                    .or_insert_with(|| StructInfo {
                        fields: BTreeMap::new(),
                        embeds: Vec::new(),
                        defaults: BTreeSet::new(),
                    });
            }
        }
    }

    fn collect_interface_names(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            if let Stmt::Interface { name, .. } = stmt {
                let iface_name = token_text(self.source, name.span);
                self.interfaces
                    .entry(iface_name)
                    .or_insert_with(|| InterfaceInfo {
                        methods: HashMap::new(),
                    });
            }
        }
    }

    fn collect_enum_names(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            let Stmt::Enum {
                name, backed_type, ..
            } = stmt
            else {
                continue;
            };
            let enum_name = token_text(self.source, name.span);
            let backed = backed_type.and_then(|ty| enum_backed_primitive(ty));
            self.enums.entry(enum_name).or_insert_with(|| EnumInfo {
                cases: BTreeMap::new(),
                backed,
            });
        }
    }

    fn collect_struct_fields(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            if let Stmt::Class { kind, name, members, .. } = stmt {
                if *kind != ClassKind::Struct {
                    continue;
                }
                let class_name = token_text(self.source, name.span);
                let mut fields = BTreeMap::new();
                let mut embeds = Vec::new();
                let mut embed_names = HashSet::new();
                let mut defaults = BTreeSet::new();
                for member in members.iter() {
                    match member {
                        ClassMember::Property { ty, entries, .. } => {
                            let field_type = ty.map(|ty| self.resolve_type(ty));
                            for entry in entries.iter() {
                                let field_name = token_text(self.source, entry.name.span);
                                let field_name = field_name.trim_start_matches('$').to_string();
                                if embed_names.contains(&field_name) {
                                    self.errors.push(TypeError {
                                        span: entry.name.span,
                                        message: format!(
                                            "Struct '{}' already embeds '{}'",
                                            class_name, field_name
                                        ),
                                    });
                                }
                                fields.insert(
                                    field_name.clone(),
                                    field_type.clone().unwrap_or(Type::Unknown),
                                );
                                if entry.default.is_some() {
                                    defaults.insert(field_name);
                                }
                            }
                        }
                        ClassMember::PropertyHook { ty, name, default, .. } => {
                            let field_name = token_text(self.source, name.span);
                            let field_name = field_name.trim_start_matches('$').to_string();
                            if embed_names.contains(&field_name) {
                                self.errors.push(TypeError {
                                    span: name.span,
                                    message: format!(
                                        "Struct '{}' already embeds '{}'",
                                        class_name, field_name
                                    ),
                                });
                            }
                            let field_type =
                                ty.map(|ty| self.resolve_type(ty)).unwrap_or(Type::Unknown);
                            fields.insert(field_name.clone(), field_type);
                            if default.is_some() {
                                defaults.insert(field_name);
                            }
                        }
                        ClassMember::Embed { types, .. } => {
                            for embed in types.iter() {
                                let embed_name = token_text(self.source, embed.span);
                                if embed_name == class_name {
                                    self.errors.push(TypeError {
                                        span: embed.span,
                                        message: "Struct cannot embed itself".to_string(),
                                    });
                                    continue;
                                }
                                if !self.structs.contains_key(&embed_name) {
                                    self.errors.push(TypeError {
                                        span: embed.span,
                                        message: format!(
                                            "Unknown embedded struct '{}'",
                                            embed_name
                                        ),
                                    });
                                    continue;
                                }
                                if fields.contains_key(&embed_name)
                                    || embed_names.contains(&embed_name)
                                {
                                    self.errors.push(TypeError {
                                        span: embed.span,
                                        message: format!(
                                            "Duplicate embedded struct '{}'",
                                            embed_name
                                        ),
                                    });
                                    continue;
                                }
                                embed_names.insert(embed_name.clone());
                                embeds.push(embed_name.clone());
                                fields.insert(embed_name.clone(), Type::Struct(embed_name));
                            }
                        }
                        _ => {}
                    }
                }
                if let Some(info) = self.structs.get_mut(&class_name) {
                    info.fields = fields;
                    info.embeds = embeds;
                    info.defaults = defaults;
                } else {
                    self.structs.insert(
                        class_name,
                        StructInfo {
                            fields,
                            embeds,
                            defaults,
                        },
                    );
                }
            }
        }
    }

    fn collect_interface_methods(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            let Stmt::Interface { name, members, .. } = stmt else {
                continue;
            };
            let iface_name = token_text(self.source, name.span);
            let mut methods = HashMap::new();
            for member in members.iter() {
                if let ClassMember::Method {
                    name: method_name,
                    params,
                    return_type,
                    ..
                } = member
                {
                    let method_name = token_text(self.source, method_name.span);
                    let sig = self.method_signature(params, *return_type);
                    methods.insert(method_name, sig);
                }
            }
            self.interfaces.insert(
                iface_name,
                InterfaceInfo {
                    methods,
                },
            );
        }
    }

    fn collect_struct_methods(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            let Stmt::Class {
                kind,
                name,
                members,
                ..
            } = stmt else {
                continue;
            };
            if *kind != ClassKind::Struct {
                continue;
            }
            let struct_name = token_text(self.source, name.span);
            let mut methods = HashMap::new();
            for member in members.iter() {
                if let ClassMember::Method {
                    name: method_name,
                    params,
                    return_type,
                    ..
                } = member
                {
                    let method_name = token_text(self.source, method_name.span);
                    let sig = self.method_signature(params, *return_type);
                    methods.insert(method_name, sig);
                }
            }
            self.struct_methods.insert(struct_name, methods);
        }
    }

    fn collect_enum_methods(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            let Stmt::Enum { name, members, .. } = stmt else {
                continue;
            };
            let enum_name = token_text(self.source, name.span);
            let mut methods = HashMap::new();
            for member in members.iter() {
                if let ClassMember::Method {
                    name: method_name,
                    params,
                    return_type,
                    ..
                } = member
                {
                    let method_name = token_text(self.source, method_name.span);
                    let sig = self.method_signature(params, *return_type);
                    methods.insert(method_name, sig);
                }
            }
            self.enum_methods.insert(enum_name, methods);
        }
    }

    fn collect_enum_cases(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            let Stmt::Enum { name, members, .. } = stmt else {
                continue;
            };
            let enum_name = token_text(self.source, name.span);
            let mut cases = BTreeMap::new();
            for member in members.iter() {
                let ClassMember::Case {
                    name: case_name,
                    payload,
                    span,
                    ..
                } = member
                else {
                    continue;
                };
                let case_name = token_text(self.source, case_name.span);
                if cases.contains_key(&case_name) {
                    self.errors.push(TypeError {
                        span: *span,
                        message: format!("Duplicate enum case '{}::{}'", enum_name, case_name),
                    });
                    continue;
                }
                let mut params = Vec::new();
                if let Some(payload) = payload {
                    let mut seen_params = HashSet::new();
                    for param in payload.iter() {
                        if param.by_ref {
                            self.errors.push(TypeError {
                                span: param.span,
                                message: "Enum case payload parameters cannot be by-reference"
                                    .to_string(),
                            });
                        }
                        if param.variadic {
                            self.errors.push(TypeError {
                                span: param.span,
                                message: "Enum case payload parameters cannot be variadic"
                                    .to_string(),
                            });
                        }
                        if param.default.is_some() {
                            self.errors.push(TypeError {
                                span: param.span,
                                message:
                                    "Enum case payload parameters cannot have default values"
                                        .to_string(),
                            });
                        }
                        let name = token_text(self.source, param.name.span);
                        let name = name.trim_start_matches('$').to_string();
                        if !seen_params.insert(name.clone()) {
                            self.errors.push(TypeError {
                                span: param.span,
                                message: format!(
                                    "Duplicate payload field '{}' on enum case {}::{}",
                                    name, enum_name, case_name
                                ),
                            });
                        }
                        let ty = param.ty.map(|ty| self.resolve_type(ty));
                        let required = param.default.is_none() && !param.variadic;
                        params.push(EnumParamInfo {
                            name,
                            ty,
                            required,
                            variadic: param.variadic,
                        });
                    }
                }
                cases.insert(case_name, EnumCaseInfo { params });
            }
            if let Some(info) = self.enums.get_mut(&enum_name) {
                info.cases = cases;
            } else {
                self.enums.insert(
                    enum_name,
                    EnumInfo {
                        cases,
                        backed: None,
                    },
                );
            }
        }
    }

    fn method_signature(
        &mut self,
        params: &'a [crate::parser::ast::Param<'a>],
        return_type: Option<&'a AstType<'a>>,
    ) -> MethodSig {
        let mut sig_params = Vec::new();
        let mut variadic = false;
        for param in params.iter() {
            let ty = param.ty.map(|ty| self.resolve_type(ty));
            let required = param.default.is_none() && !param.variadic;
            if param.variadic {
                variadic = true;
            }
            sig_params.push(ParamSig { ty, required });
        }
        MethodSig {
            params: sig_params,
            return_type: return_type.map(|ty| self.resolve_type(ty)),
            variadic,
        }
    }

    fn collect_functions(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            if let Stmt::Function {
                name,
                type_params,
                params,
                return_type,
                ..
            } = stmt
            {
                let fn_name = token_text(self.source, name.span);
                let (type_param_sigs, type_param_set) =
                    self.collect_type_param_sigs(type_params);
                let mut param_sigs = Vec::new();
                let mut variadic = false;
                for param in params.iter() {
                    let ty = param
                        .ty
                        .map(|ty| self.resolve_type_with_params(ty, &type_param_set));
                    let required = param.default.is_none() && !param.variadic;
                    if param.variadic {
                        variadic = true;
                    }
                    param_sigs.push(ParamSig { ty, required });
                }
                let sig = FunctionSig {
                    type_params: type_param_sigs,
                    params: param_sigs,
                    return_type: return_type
                        .map(|ty| self.resolve_type_with_params(ty, &type_param_set)),
                    variadic,
                };
                if let Some(ret) = &sig.return_type {
                    self.function_returns.insert(fn_name.clone(), ret.clone());
                }
                self.functions.insert(fn_name, sig);
            }
        }
    }

    fn collect_type_aliases(&mut self, program: &Program<'a>) {
        for stmt in program.statements.iter() {
            let Stmt::TypeAlias {
                name,
                type_params,
                ty,
                span,
            } = stmt
            else {
                continue;
            };
            let alias_name = token_text(self.source, name.span);
            if is_builtin_type_name(&alias_name) {
                self.errors.push(TypeError {
                    span: *span,
                    message: format!("Type alias '{}' shadows a builtin type", alias_name),
                });
                continue;
            }
            if self.structs.contains_key(&alias_name) {
                self.errors.push(TypeError {
                    span: *span,
                    message: format!("Type alias '{}' conflicts with struct name", alias_name),
                });
                continue;
            }
            if self.enums.contains_key(&alias_name) {
                self.errors.push(TypeError {
                    span: *span,
                    message: format!("Type alias '{}' conflicts with enum name", alias_name),
                });
                continue;
            }
            if self.type_aliases.contains_key(&alias_name) {
                self.errors.push(TypeError {
                    span: *span,
                    message: format!("Duplicate type alias '{}'", alias_name),
                });
                continue;
            }
            let (param_sigs, param_set) = self.collect_type_param_sigs(type_params);
            let resolved_body = self.resolve_type_with_params(ty, &param_set);
            self.type_aliases.insert(
                alias_name,
                TypeAliasInfo {
                    params: param_sigs,
                    ty: resolved_body,
                    span: *span,
                },
            );
        }
    }

    fn collect_type_param_sigs(
        &mut self,
        params: &'a [TypeParam<'a>],
    ) -> (Vec<TypeParamSig>, HashSet<String>) {
        if params.is_empty() {
            return (Vec::new(), HashSet::new());
        }
        let mut seen = HashSet::new();
        let mut names = Vec::new();
        for param in params.iter() {
            let name = token_text(self.source, param.name.span);
            if !seen.insert(name.clone()) {
                self.errors.push(TypeError {
                    span: param.span,
                    message: format!("Duplicate type parameter '{}'", name),
                });
            }
            names.push(name);
        }
        let param_set: HashSet<String> = names.iter().cloned().collect();
        let mut out = Vec::new();
        for (idx, param) in params.iter().enumerate() {
            let constraint = param
                .constraint
                .map(|ty| self.resolve_type_with_params(ty, &param_set));
            out.push(TypeParamSig {
                name: names[idx].clone(),
                constraint,
            });
        }
        (out, param_set)
    }

    fn check_call_signature(
        &mut self,
        func: ExprId<'a>,
        args: &'a [crate::parser::ast::Arg<'a>],
        env: &HashMap<String, Type>,
    ) -> Type {
        let Expr::Variable { span, .. } = *func else {
            return Type::Unknown;
        };
        let name = token_text(self.source, span);
        if name.starts_with('$') {
            return Type::Unknown;
        }
        let Some(sig) = self.functions.get(&name) else {
            return Type::Unknown;
        };
        let sig = sig.clone();

        let required = sig.params.iter().filter(|p| p.required).count();
        if args.len() < required {
            self.errors.push(TypeError {
                span: Span::new(span.start, span.end),
                message: format!(
                    "Missing arguments for {}(): expected at least {}, got {}",
                    name,
                    required,
                    args.len()
                ),
            });
        }

        let mut actuals = Vec::new();
        for arg in args.iter() {
            actuals.push(self.infer_expr_with_env(arg.value, env));
        }

        let mut inferred = HashMap::new();
        if !sig.type_params.is_empty() {
            let mut idx = 0;
            while idx < args.len() {
                let param_ty = if idx >= sig.params.len() {
                    if sig.variadic {
                        sig.params.last().and_then(|p| p.ty.as_ref())
                    } else {
                        None
                    }
                } else {
                    sig.params[idx].ty.as_ref()
                };
                if let Some(param_ty) = param_ty {
                    self.infer_type_params(param_ty, &actuals[idx], &mut inferred);
                }
                idx += 1;
            }

            for param in sig.type_params.iter() {
                if !inferred.contains_key(&param.name) {
                    self.errors.push(TypeError {
                        span: Span::new(span.start, span.end),
                        message: format!(
                            "Unable to infer type parameter '{}' for {}()",
                            param.name, name
                        ),
                    });
                }
            }

            for param in sig.type_params.iter() {
                let Some(inferred_ty) = inferred.get(&param.name) else {
                    continue;
                };
                if let Some(constraint) = &param.constraint {
                    if !self.is_assignable(inferred_ty, constraint) {
                        self.errors.push(TypeError {
                            span: Span::new(span.start, span.end),
                            message: format!(
                                "Type argument for '{}' does not satisfy constraint {}",
                                param.name, constraint
                            ),
                        });
                    }
                }
            }
        }

        let mut idx = 0;
        while idx < args.len() {
            let param_ty = if idx >= sig.params.len() {
                if sig.variadic {
                    sig.params.last().and_then(|p| p.ty.as_ref())
                } else {
                    None
                }
            } else {
                sig.params[idx].ty.as_ref()
            };
            if let Some(param_ty) = param_ty {
                let expected = substitute_type(param_ty, &inferred);
                if matches!(actuals[idx], Type::Primitive(PrimitiveType::Null))
                    && !self.type_allows_null(&expected)
                {
                    self.errors.push(TypeError {
                        span: args[idx].span,
                        message: "Null is not allowed in PHPX; use Option<T> instead".to_string(),
                    });
                }
                if let Expr::ObjectLiteral { items, span } = *args[idx].value {
                    self.check_object_literal_against_type(items, &expected, span, env);
                }
                if !self.is_assignable(&actuals[idx], &expected) {
                    self.errors.push(TypeError {
                        span: args[idx].span,
                        message: format!(
                            "Argument {} type mismatch: expected {}, got {}",
                            idx + 1,
                            expected,
                            actuals[idx]
                        ),
                    });
                }
            } else if matches!(actuals[idx], Type::Primitive(PrimitiveType::Null)) {
                self.errors.push(TypeError {
                    span: args[idx].span,
                    message: "Null is not allowed in PHPX; use Option<T> instead".to_string(),
                });
            }
            idx += 1;
        }

        let ret = sig.return_type.clone().unwrap_or(Type::Unknown);
        substitute_type(&ret, &inferred)
    }

    fn check_method_call_signature(
        &mut self,
        target_ty: &Type,
        method: ExprId<'a>,
        args: &'a [crate::parser::ast::Arg<'a>],
        env: &HashMap<String, Type>,
        span: Span,
    ) -> Type {
        let Some(method_name) = self.extract_static_ident(method) else {
            return Type::Unknown;
        };

        let (owner_label, sig) = match target_ty {
            Type::Struct(name) => (
                Some(format!("struct {}", name)),
                self.struct_methods
                    .get(name)
                    .and_then(|methods| methods.get(&method_name))
                    .cloned(),
            ),
            Type::Interface(name) => (
                Some(format!("interface {}", name)),
                self.interfaces
                    .get(name)
                    .and_then(|info| info.methods.get(&method_name))
                    .cloned(),
            ),
            Type::Enum(name) => (
                Some(format!("enum {}", name)),
                self.enum_methods
                    .get(name)
                    .and_then(|methods| methods.get(&method_name))
                    .cloned(),
            ),
            Type::EnumCase { enum_name, .. } => (
                Some(format!("enum {}", enum_name)),
                self.enum_methods
                    .get(enum_name)
                    .and_then(|methods| methods.get(&method_name))
                    .cloned(),
            ),
            _ => (None, None),
        };

        let Some(sig) = sig else {
            if let Some(owner) = owner_label {
                self.errors.push(TypeError {
                    span,
                    message: format!("Unknown method '{}' on {}", method_name, owner),
                });
            }
            return Type::Unknown;
        };

        let required = sig.params.iter().filter(|p| p.required).count();
        if args.len() < required {
            self.errors.push(TypeError {
                span,
                message: format!(
                    "Missing arguments for {}(): expected at least {}, got {}",
                    method_name,
                    required,
                    args.len()
                ),
            });
        }

        let mut actuals = Vec::new();
        for arg in args.iter() {
            actuals.push(self.infer_expr_with_env(arg.value, env));
        }

        let mut idx = 0;
        while idx < args.len() {
            let param_ty = if idx >= sig.params.len() {
                if sig.variadic {
                    sig.params.last().and_then(|p| p.ty.as_ref())
                } else {
                    None
                }
            } else {
                sig.params[idx].ty.as_ref()
            };
            if let Some(param_ty) = param_ty {
                if matches!(actuals[idx], Type::Primitive(PrimitiveType::Null))
                    && !self.type_allows_null(param_ty)
                {
                    self.errors.push(TypeError {
                        span: args[idx].span,
                        message: "Null is not allowed in PHPX; use Option<T> instead".to_string(),
                    });
                }
                if let Expr::ObjectLiteral { items, span } = *args[idx].value {
                    self.check_object_literal_against_type(items, param_ty, span, env);
                }
                if !self.is_assignable(&actuals[idx], param_ty) {
                    self.errors.push(TypeError {
                        span: args[idx].span,
                        message: format!(
                            "Argument {} type mismatch: expected {}, got {}",
                            idx + 1,
                            param_ty,
                            actuals[idx]
                        ),
                    });
                }
            } else if matches!(actuals[idx], Type::Primitive(PrimitiveType::Null)) {
                self.errors.push(TypeError {
                    span: args[idx].span,
                    message: "Null is not allowed in PHPX; use Option<T> instead".to_string(),
                });
            }
            idx += 1;
        }

        sig.return_type.clone().unwrap_or(Type::Unknown)
    }

    fn check_struct_defaults(&mut self, members: &'a [ClassMember<'a>]) {
        for member in members.iter() {
            match member {
                ClassMember::Property { ty, entries, .. } => {
                    let expected = ty.map(|ty| self.resolve_type(ty));
                    for entry in entries.iter() {
                        self.check_property_default(entry, expected.as_ref());
                    }
                }
                ClassMember::PropertyHook { ty, name, default, .. } => {
                    if let Some(expected) = ty.map(|ty| self.resolve_type(ty)) {
                        if let Some(default) = default {
                            if !self.is_constant_expr(default) {
                                self.errors.push(TypeError {
                                    span: member_span(member),
                                    message:
                                        "Struct field defaults must be constant expressions"
                                            .to_string(),
                                });
                            }
                            let actual = self.infer_expr_with_env(*default, &HashMap::new());
                            if !self.is_assignable(&actual, &expected) {
                                let prop_name = token_text(self.source, name.span);
                                self.errors.push(TypeError {
                                    span: member_span(member),
                                    message: format!(
                                        "Default value for {} has type {}, expected {}",
                                        prop_name, actual, expected
                                    ),
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn check_property_default(
        &mut self,
        entry: &PropertyEntry<'a>,
        expected: Option<&Type>,
    ) {
        let Some(expected) = expected else {
            return;
        };
        let Some(default) = entry.default else {
            return;
        };
        if !self.is_constant_expr(default) {
            self.errors.push(TypeError {
                span: entry.span,
                message: "Struct field defaults must be constant expressions".to_string(),
            });
        }
        let actual = self.infer_expr_with_env(default, &HashMap::new());
        if !self.is_assignable(&actual, expected) {
            let name = token_text(self.source, entry.name.span);
            self.errors.push(TypeError {
                span: entry.span,
                message: format!(
                    "Default value for {} has type {}, expected {}",
                    name, actual, expected
                ),
            });
        }
    }

    fn is_constant_expr(&self, expr: &Expr<'a>) -> bool {
        match expr {
            Expr::Integer { .. }
            | Expr::Float { .. }
            | Expr::String { .. }
            | Expr::Boolean { .. }
            | Expr::Null { .. } => true,
            Expr::Array { items, .. } => items.iter().all(|item| {
                if item.unpack {
                    return false;
                }
                let key_ok = item
                    .key
                    .map(|key| self.is_constant_expr(key))
                    .unwrap_or(true);
                key_ok && self.is_constant_expr(item.value)
            }),
            Expr::ObjectLiteral { items, .. } => {
                items.iter().all(|item| self.is_constant_expr(item.value))
            }
            Expr::StructLiteral { fields, .. } => {
                fields.iter().all(|field| self.is_constant_expr(field.value))
            }
            Expr::ClassConstFetch { .. } => true,
            Expr::Binary { op, left, right, .. } => {
                matches!(op, BinaryOp::BitOr)
                    && self.is_constant_expr(left)
                    && self.is_constant_expr(right)
            }
            Expr::Unary { op, expr, .. } => {
                matches!(op, UnaryOp::Plus | UnaryOp::Minus) && self.is_constant_expr(expr)
            }
            _ => false,
        }
    }

    fn resolve_type(&mut self, ty: &AstType<'a>) -> Type {
        let mut visiting = HashSet::new();
        let params = HashSet::new();
        self.resolve_type_internal(ty, &mut visiting, &params)
    }

    fn resolve_type_with_params(&mut self, ty: &AstType<'a>, params: &HashSet<String>) -> Type {
        let mut visiting = HashSet::new();
        self.resolve_type_internal(ty, &mut visiting, params)
    }

    fn resolve_type_internal(
        &mut self,
        ty: &AstType<'a>,
        visiting: &mut HashSet<String>,
        params: &HashSet<String>,
    ) -> Type {
        match ty {
            AstType::Simple(token) => {
                if token.kind == TokenKind::TypeNull {
                    self.errors.push(TypeError {
                        span: token.span,
                        message: "Null types are not allowed in PHPX; use Option<T> instead"
                            .to_string(),
                    });
                }
                self.resolve_named_type(token_text(self.source, token.span), visiting, params)
            }
            AstType::Name(name) => self.resolve_name_type(name, visiting, params),
            AstType::Union(types) => {
                if let Some(span) = self.find_null_type_span(types) {
                    self.errors.push(TypeError {
                        span,
                        message: "Nullable unions are not allowed in PHPX; use Option<T> instead"
                            .to_string(),
                    });
                }
                let mut out = Vec::new();
                for ty in types.iter() {
                    out.push(self.resolve_type_internal(ty, visiting, params));
                }
                Type::Union(out)
            }
            AstType::Intersection(types) => {
                let mut out = Vec::new();
                for ty in types.iter() {
                    out.push(self.resolve_type_internal(ty, visiting, params));
                }
                if out.is_empty() {
                    Type::Unknown
                } else if out.len() == 1 {
                    out[0].clone()
                } else {
                    Type::Union(out)
                }
            }
            AstType::Nullable(inner) => {
                self.errors.push(TypeError {
                    span: self.type_span(inner),
                    message: "Nullable types are not allowed in PHPX; use Option<T> instead"
                        .to_string(),
                });
                let inner = self.resolve_type_internal(inner, visiting, params);
                Type::Union(vec![inner, Type::Primitive(PrimitiveType::Null)])
            }
            AstType::ObjectShape(fields) => {
                let mut map = BTreeMap::new();
                for field in fields.iter() {
                    let name = parse_type_field_name(self.source, field.name.span);
                    let ty = self.resolve_type_internal(field.ty, visiting, params);
                    map.insert(
                        name,
                        ObjectField {
                            ty,
                            optional: field.optional,
                        },
                    );
                }
                Type::ObjectShape(map)
            }
            AstType::Applied { base, args } => {
                let base_name = match self.base_type_name(base) {
                    Some(name) => name,
                    None => return Type::Unknown,
                };
                if base_name.eq_ignore_ascii_case("Option") && args.len() != 1 {
                    self.errors.push(TypeError {
                        span: self.type_span(base),
                        message: "Option<T> expects exactly one type argument".to_string(),
                    });
                }
                if base_name.eq_ignore_ascii_case("Result") && args.len() != 2 {
                    self.errors.push(TypeError {
                        span: self.type_span(base),
                        message: "Result<T, E> expects exactly two type arguments".to_string(),
                    });
                }
                if base_name.eq_ignore_ascii_case("array") && args.len() != 1 {
                    self.errors.push(TypeError {
                        span: self.type_span(base),
                        message: "array<T> expects exactly one type argument".to_string(),
                    });
                }
                let mut resolved_args = Vec::new();
                for arg in args.iter() {
                    resolved_args.push(self.resolve_type_internal(arg, visiting, params));
                }
                if let Some(instantiated) =
                    self.resolve_alias_applied(&base_name, &resolved_args, visiting)
                {
                    instantiated
                } else {
                    if !base_name.eq_ignore_ascii_case("Option")
                        && !base_name.eq_ignore_ascii_case("Result")
                        && !base_name.eq_ignore_ascii_case("array")
                    {
                        self.errors.push(TypeError {
                            span: self.type_span(base),
                            message: format!(
                                "Unknown generic type '{}' in PHPX; classes are not allowed",
                                base_name
                            ),
                        });
                        Type::Unknown
                    } else {
                        Type::Applied {
                            base: base_name,
                            args: resolved_args,
                        }
                    }
                }
            }
        }
    }

    fn type_span(&self, ty: &AstType<'a>) -> Span {
        match ty {
            AstType::Simple(token) => token.span,
            AstType::Name(name) => name.parts.first().map(|p| p.span).unwrap_or_default(),
            AstType::Union(types)
            | AstType::Intersection(types) => types
                .first()
                .map(|t| self.type_span(t))
                .unwrap_or_default(),
            AstType::Nullable(inner) => self.type_span(inner),
            AstType::ObjectShape(fields) => fields
                .first()
                .map(|field| field.span)
                .unwrap_or_default(),
            AstType::Applied { base, .. } => self.type_span(base),
        }
    }

    fn find_null_type_span(&self, types: &'a [AstType<'a>]) -> Option<Span> {
        for ty in types.iter() {
            match ty {
                AstType::Simple(token) if token.kind == TokenKind::TypeNull => {
                    return Some(token.span);
                }
                AstType::Nullable(inner) => return Some(self.type_span(inner)),
                AstType::Union(inner) | AstType::Intersection(inner) => {
                    if let Some(span) = self.find_null_type_span(inner) {
                        return Some(span);
                    }
                }
                _ => {}
            }
        }
        None
    }

    fn resolve_named_type(
        &mut self,
        name: String,
        visiting: &mut HashSet<String>,
        params: &HashSet<String>,
    ) -> Type {
        let lower = name.to_ascii_lowercase();
        match lower.as_str() {
            "int" | "integer" => Type::Primitive(PrimitiveType::Int),
            "float" | "double" => Type::Primitive(PrimitiveType::Float),
            "bool" | "boolean" => Type::Primitive(PrimitiveType::Bool),
            "string" => Type::Primitive(PrimitiveType::String),
            "null" => Type::Primitive(PrimitiveType::Null),
            "array" => Type::Array,
            "object" => Type::Object,
            "mixed" => Type::Mixed,
            _ => {
                if params.contains(&name) {
                    Type::TypeParam(name)
                } else if let Some(alias) = self.resolve_alias(&name, visiting) {
                    alias
                } else if name.eq_ignore_ascii_case("Option") {
                    self.errors.push(TypeError {
                        span: Span::new(0, 0),
                        message: "Option<T> requires a type argument".to_string(),
                    });
                    Type::Unknown
                } else if name.eq_ignore_ascii_case("Result") {
                    self.errors.push(TypeError {
                        span: Span::new(0, 0),
                        message: "Result<T, E> requires type arguments".to_string(),
                    });
                    Type::Unknown
                } else if self.enums.contains_key(&name) {
                    Type::Enum(name)
                } else if self.interfaces.contains_key(&name) {
                    Type::Interface(name)
                } else if self.structs.contains_key(&name) {
                    Type::Struct(name)
                } else {
                    Type::Object
                }
            }
        }
    }

    fn resolve_name_type(
        &mut self,
        name: &Name<'a>,
        visiting: &mut HashSet<String>,
        params: &HashSet<String>,
    ) -> Type {
        let mut out = String::new();
        for (idx, part) in name.parts.iter().enumerate() {
            let text = token_text(self.source, part.span);
            if idx > 0 {
                out.push('\\');
            }
            out.push_str(text.trim_matches('\\'));
        }
        if out.eq_ignore_ascii_case("Option") {
            self.errors.push(TypeError {
                span: name.span,
                message: "Option<T> requires a type argument".to_string(),
            });
            return Type::Unknown;
        }
        if out.eq_ignore_ascii_case("Result") {
            self.errors.push(TypeError {
                span: name.span,
                message: "Result<T, E> requires type arguments".to_string(),
            });
            return Type::Unknown;
        }
        if !self.is_known_named_type(&out, params) {
            self.errors.push(TypeError {
                span: name.span,
                message: format!(
                    "Unknown type '{}' in PHPX; classes are not allowed",
                    out
                ),
            });
            return Type::Unknown;
        }
        self.resolve_named_type(out, visiting, params)
    }

    fn resolve_alias(&mut self, name: &str, visiting: &mut HashSet<String>) -> Option<Type> {
        if let Some(resolved) = self.resolved_aliases.get(name) {
            return Some(resolved.clone());
        }
        let info = match self.type_aliases.get(name) {
            Some(info) => info,
            None => return None,
        };
        if !info.params.is_empty() {
            self.errors.push(TypeError {
                span: info.span,
                message: format!("Type alias '{}' requires type arguments", name),
            });
            return Some(Type::Unknown);
        }
        if !visiting.insert(name.to_string()) {
            self.errors.push(TypeError {
                span: info.span,
                message: format!("Recursive type alias '{}'", name),
            });
            return Some(Type::Unknown);
        }
        let resolved = info.ty.clone();
        visiting.remove(name);
        self.resolved_aliases.insert(name.to_string(), resolved.clone());
        Some(resolved)
    }

    fn is_known_named_type(&self, name: &str, params: &HashSet<String>) -> bool {
        if params.contains(name) {
            return true;
        }
        if name.eq_ignore_ascii_case("int")
            || name.eq_ignore_ascii_case("integer")
            || name.eq_ignore_ascii_case("float")
            || name.eq_ignore_ascii_case("double")
            || name.eq_ignore_ascii_case("bool")
            || name.eq_ignore_ascii_case("boolean")
            || name.eq_ignore_ascii_case("string")
            || name.eq_ignore_ascii_case("null")
            || name.eq_ignore_ascii_case("array")
            || name.eq_ignore_ascii_case("object")
            || name.eq_ignore_ascii_case("mixed")
            || name.eq_ignore_ascii_case("option")
            || name.eq_ignore_ascii_case("result")
        {
            return true;
        }
        self.type_aliases.contains_key(name)
            || self.structs.contains_key(name)
            || self.enums.contains_key(name)
            || self.interfaces.contains_key(name)
    }

    fn resolve_alias_applied(
        &mut self,
        name: &str,
        args: &[Type],
        visiting: &mut HashSet<String>,
    ) -> Option<Type> {
        let info = self.type_aliases.get(name)?;
        if info.params.len() != args.len() {
            self.errors.push(TypeError {
                span: info.span,
                message: format!(
                    "Type alias '{}' expects {} type arguments, got {}",
                    name,
                    info.params.len(),
                    args.len()
                ),
            });
            return Some(Type::Unknown);
        }
        if !visiting.insert(name.to_string()) {
            self.errors.push(TypeError {
                span: info.span,
                message: format!("Recursive type alias '{}'", name),
            });
            return Some(Type::Unknown);
        }
        let mut mapping = HashMap::new();
        for (idx, param) in info.params.iter().enumerate() {
            let arg = args[idx].clone();
            if let Some(constraint) = &param.constraint {
                if !self.is_assignable(&arg, constraint) {
                    self.errors.push(TypeError {
                        span: info.span,
                        message: format!(
                            "Type argument {} for '{}' does not satisfy constraint {}",
                            idx + 1,
                            name,
                            constraint
                        ),
                    });
                }
            }
            mapping.insert(param.name.clone(), arg);
        }
        let resolved = substitute_type(&info.ty, &mapping);
        visiting.remove(name);
        Some(resolved)
    }

    fn base_type_name(&self, base: &AstType<'a>) -> Option<String> {
        match base {
            AstType::Simple(token) => Some(token_text(self.source, token.span)),
            AstType::Name(name) => {
                let mut out = String::new();
                for (idx, part) in name.parts.iter().enumerate() {
                    let text = token_text(self.source, part.span);
                    if idx > 0 {
                        out.push('\\');
                    }
                    out.push_str(text.trim_matches('\\'));
                }
                Some(out)
            }
            _ => None,
        }
    }

    fn check_wasm_stubs(&mut self) {
        let Some(file_path) = self.file_path.as_ref() else {
            return;
        };
        let Some(modules_root) = find_modules_root(file_path) else {
            return;
        };

        let source = String::from_utf8_lossy(self.source);
        let regex = match Regex::new(
            r#"(?m)^[\t \r]*import\s+\{[^}]+\}\s+from\s+['"]([^'"]+)['"]\s*(?:as\s+([A-Za-z_][A-Za-z0-9_]*))?\s*;?\s*$"#,
        ) {
            Ok(regex) => regex,
            Err(_) => return,
        };

        for caps in regex.captures_iter(&source) {
            let kind = caps.get(2).map(|m| m.as_str());
            if kind != Some("wasm") {
                continue;
            }
            let Some(matched) = caps.get(0) else {
                continue;
            };
            let Some(spec) = caps.get(1).map(|m| m.as_str()) else {
                continue;
            };

            if let Err(message) = resolve_wasm_stub(spec, file_path, &modules_root) {
                self.errors.push(TypeError {
                    span: Span::new(matched.start(), matched.end()),
                    message,
                });
            }
        }
    }
}

fn member_span(member: &ClassMember) -> Span {
    match member {
        ClassMember::Property { span, .. }
        | ClassMember::PropertyHook { span, .. }
        | ClassMember::Method { span, .. }
        | ClassMember::Const { span, .. }
        | ClassMember::TraitUse { span, .. }
        | ClassMember::Embed { span, .. }
        | ClassMember::Case { span, .. } => *span,
    }
}

fn enum_backed_primitive(ty: &AstType<'_>) -> Option<PrimitiveType> {
    match ty {
        AstType::Simple(token) => match token.kind {
            TokenKind::TypeInt => Some(PrimitiveType::Int),
            TokenKind::TypeString => Some(PrimitiveType::String),
            _ => None,
        },
        _ => None,
    }
}

impl<'a> CheckContext<'a> {
    fn is_assignable(&self, source: &Type, target: &Type) -> bool {
        if matches!(target, Type::Unknown | Type::Mixed)
            || matches!(source, Type::Unknown | Type::Mixed)
        {
            return true;
        }
        if matches!(target, Type::TypeParam(_)) || matches!(source, Type::TypeParam(_)) {
            return true;
        }
        match target {
            Type::Union(options) => {
                return options.iter().any(|opt| self.is_assignable(source, opt));
            }
            _ => {}
        }
        match source {
            Type::Union(options) => {
                return options.iter().all(|opt| self.is_assignable(opt, target));
            }
            _ => {}
        }
        if let Type::Applied { base, args } = target {
            if base.eq_ignore_ascii_case("Option") && args.len() == 1 {
                if let Type::EnumCase {
                    enum_name,
                    case_name,
                    args: source_args,
                } = source
                {
                    if enum_name.eq_ignore_ascii_case("Option") {
                        if case_name.eq_ignore_ascii_case("None") {
                            return true;
                        }
                        if case_name.eq_ignore_ascii_case("Some") {
                            let actual = source_args.get(0).unwrap_or(&Type::Unknown);
                            return self.is_assignable(actual, &args[0]);
                        }
                    }
                }
            }
            if base.eq_ignore_ascii_case("Result") && args.len() == 2 {
                if let Type::EnumCase {
                    enum_name,
                    case_name,
                    args: source_args,
                } = source
                {
                    if enum_name.eq_ignore_ascii_case("Result") {
                        if case_name.eq_ignore_ascii_case("Ok") {
                            let actual = source_args.get(0).unwrap_or(&Type::Unknown);
                            return self.is_assignable(actual, &args[0]);
                        }
                        if case_name.eq_ignore_ascii_case("Err") {
                            let actual = source_args.get(1).unwrap_or(&Type::Unknown);
                            return self.is_assignable(actual, &args[1]);
                        }
                    }
                }
            }
        }
        match target {
            Type::Interface(name) => {
                return self.type_satisfies_interface(source, name);
            }
            _ => {}
        }
        match (source, target) {
            (Type::Interface(a), Type::Interface(b)) => a == b,
            (Type::Interface(_), Type::Object) => true,
            _ => is_assignable_base(source, target),
        }
    }

    fn type_satisfies_interface(&self, source: &Type, iface: &str) -> bool {
        let Some(info) = self.interfaces.get(iface) else {
            return false;
        };
        match source {
            Type::Struct(name) => self.struct_satisfies_interface(name, info),
            Type::Enum(name) => self.enum_satisfies_interface(name, info),
            Type::EnumCase { enum_name, .. } => self.enum_satisfies_interface(enum_name, info),
            Type::Interface(name) => name.eq_ignore_ascii_case(iface),
            _ => false,
        }
    }

    fn struct_satisfies_interface(&self, name: &str, iface: &InterfaceInfo) -> bool {
        let Some(methods) = self.struct_methods.get(name) else {
            return false;
        };
        for (method_name, expected) in iface.methods.iter() {
            let Some(actual) = methods.get(method_name) else {
                return false;
            };
            if !self.method_satisfies_interface(actual, expected) {
                return false;
            }
        }
        true
    }

    fn enum_satisfies_interface(&self, name: &str, iface: &InterfaceInfo) -> bool {
        let Some(methods) = self.enum_methods.get(name) else {
            return false;
        };
        for (method_name, expected) in iface.methods.iter() {
            let Some(actual) = methods.get(method_name) else {
                return false;
            };
            if !self.method_satisfies_interface(actual, expected) {
                return false;
            }
        }
        true
    }

    fn method_satisfies_interface(&self, actual: &MethodSig, expected: &MethodSig) -> bool {
        if expected.variadic && !actual.variadic {
            return false;
        }

        let expected_required = expected.params.iter().filter(|p| p.required).count();
        let actual_required = actual.params.iter().filter(|p| p.required).count();
        if actual_required > expected_required {
            return false;
        }

        if expected.params.len() > actual.params.len() && !actual.variadic {
            return false;
        }

        let actual_last = actual.params.last();
        for (idx, expected_param) in expected.params.iter().enumerate() {
            let actual_param = if idx < actual.params.len() {
                &actual.params[idx]
            } else {
                match actual_last {
                    Some(param) => param,
                    None => return false,
                }
            };

            let expected_ty = expected_param
                .ty
                .as_ref()
                .cloned()
                .unwrap_or(Type::Mixed);
            let actual_ty = actual_param
                .ty
                .as_ref()
                .cloned()
                .unwrap_or(Type::Mixed);
            if !self.is_assignable(&expected_ty, &actual_ty) {
                return false;
            }
        }

        if let Some(expected_ret) = expected.return_type.as_ref() {
            let actual_ret = actual
                .return_type
                .as_ref()
                .cloned()
                .unwrap_or(Type::Mixed);
            if !self.is_assignable(&actual_ret, expected_ret) {
                return false;
            }
        }
        true
    }
}

fn is_assignable_base(source: &Type, target: &Type) -> bool {
    match target {
        Type::Union(options) => {
            return options.iter().any(|opt| is_assignable_base(source, opt));
        }
        _ => {}
    }
    match source {
        Type::Union(options) => {
            return options
                .iter()
                .all(|opt| is_assignable_base(opt, target));
        }
        _ => {}
    }
        match (source, target) {
            (Type::Primitive(a), Type::Primitive(b)) => match (a, b) {
                (PrimitiveType::Int, PrimitiveType::Float) => true,
                _ => a == b,
            },
        (Type::Array, Type::Array) => true,
        (Type::VNode, Type::VNode) => true,
        (Type::Struct(a), Type::Struct(b)) => a == b,
        (Type::Enum(a), Type::Enum(b)) => a == b,
        (
            Type::EnumCase { enum_name, .. },
            Type::Enum(target_name),
        ) => enum_name == target_name,
        (
            Type::EnumCase {
                enum_name: a_enum,
                case_name: a_case,
                ..
            },
            Type::EnumCase {
                enum_name: b_enum,
                case_name: b_case,
                ..
            },
        ) => a_enum == b_enum && a_case == b_case,
        (Type::ObjectShape(fields), Type::ObjectShape(expected)) => {
            expected.iter().all(|(name, expected_field)| {
                match fields.get(name) {
                    Some(actual_field) => {
                        if actual_field.optional && !expected_field.optional {
                            return false;
                        }
                        is_assignable_base(&actual_field.ty, &expected_field.ty)
                    }
                    None => expected_field.optional,
                }
            })
        }
        (
            Type::Applied {
                base: base_a,
                args: args_a,
            },
            Type::Applied {
                base: base_b,
                args: args_b,
            },
        ) => {
            base_a.eq_ignore_ascii_case(base_b)
                && args_a.len() == args_b.len()
                && args_a
                    .iter()
                    .zip(args_b.iter())
                    .all(|(a, b)| is_assignable_base(a, b))
        }
        (Type::Array, Type::Applied { base, .. }) if base.eq_ignore_ascii_case("array") => true,
        (Type::Applied { base, .. }, Type::Array) if base.eq_ignore_ascii_case("array") => true,
        (Type::ObjectShape(_), Type::Object)
        | (Type::Struct(_), Type::Object)
        | (Type::Enum(_), Type::Object)
        | (Type::EnumCase { .. }, Type::Object)
        | (Type::VNode, Type::Object)
        | (Type::Object, Type::Object)
        | (Type::Interface(_), Type::Object) => true,
        _ => false,
    }
}

fn token_text(source: &[u8], span: Span) -> String {
    let start = span.start;
    let end = span.end.min(source.len());
    String::from_utf8_lossy(&source[start..end]).to_string()
}

fn parse_type_field_name(source: &[u8], span: Span) -> String {
    let raw = token_text(source, span);
    if raw.len() >= 2 {
        let bytes = raw.as_bytes();
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            let inner = &raw[1..raw.len() - 1];
            return unescape_type_string(inner, first == b'"');
        }
    }
    raw
}

fn unescape_type_string(value: &str, double_quoted: bool) -> String {
    let mut out = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            out.push('\\');
            break;
        };
        match next {
            '\'' if !double_quoted => out.push('\''),
            '"' if double_quoted => out.push('"'),
            '\\' => out.push('\\'),
            'n' if double_quoted => out.push('\n'),
            'r' if double_quoted => out.push('\r'),
            't' if double_quoted => out.push('\t'),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

fn substitute_type(ty: &Type, mapping: &HashMap<String, Type>) -> Type {
    match ty {
        Type::TypeParam(name) => mapping
            .get(name)
            .cloned()
            .unwrap_or_else(|| Type::TypeParam(name.clone())),
        Type::Union(types) => {
            let out = types
                .iter()
                .map(|t| substitute_type(t, mapping))
                .collect::<Vec<_>>();
            Type::Union(out)
        }
        Type::ObjectShape(fields) => {
            let mut out = BTreeMap::new();
            for (name, field) in fields.iter() {
                out.insert(
                    name.clone(),
                    ObjectField {
                        ty: substitute_type(&field.ty, mapping),
                        optional: field.optional,
                    },
                );
            }
            Type::ObjectShape(out)
        }
        Type::Applied { base, args } => Type::Applied {
            base: base.clone(),
            args: args.iter().map(|t| substitute_type(t, mapping)).collect(),
        },
        _ => ty.clone(),
    }
}

fn is_builtin_type_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "int"
            | "integer"
            | "float"
            | "double"
            | "bool"
            | "boolean"
            | "string"
            | "null"
            | "array"
            | "object"
            | "mixed"
            | "void"
            | "never"
            | "false"
            | "true"
            | "iterable"
            | "callable"
            | "option"
            | "result"
    )
}

fn remove_null(ty: &Type) -> Type {
    match ty {
        Type::Union(types) => {
            let mut out: Vec<Type> = types
                .iter()
                .filter(|t| !matches!(t, Type::Primitive(PrimitiveType::Null)))
                .cloned()
                .collect();
            if out.is_empty() {
                Type::Unknown
            } else if out.len() == 1 {
                out.remove(0)
            } else {
                Type::Union(out)
            }
        }
        Type::Primitive(PrimitiveType::Null) => Type::Unknown,
        _ => ty.clone(),
    }
}

fn keep_only_null(ty: &Type) -> Type {
    match ty {
        Type::Primitive(PrimitiveType::Null) => Type::Primitive(PrimitiveType::Null),
        Type::Union(types) => {
            if types
                .iter()
                .any(|t| matches!(t, Type::Primitive(PrimitiveType::Null)))
            {
                Type::Primitive(PrimitiveType::Null)
            } else {
                ty.clone()
            }
        }
        _ => ty.clone(),
    }
}

fn object_key_name(key: ObjectKey, source: &[u8]) -> String {
    match key {
        ObjectKey::Ident(token) => token_text(source, token.span),
        ObjectKey::String(token) => {
            let raw = token_text(source, token.span);
            parse_string_key(&raw)
        }
    }
}

fn parse_string_key(raw: &str) -> String {
    if raw.len() >= 2 {
        let bytes = raw.as_bytes();
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
            let inner = &raw[1..raw.len() - 1];
            return unescape_string_key(inner, first == b'"');
        }
    }
    raw.to_string()
}

fn unescape_string_key(value: &str, double_quoted: bool) -> String {
    let mut out = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            out.push('\\');
            break;
        };
        match next {
            '\'' if !double_quoted => out.push('\''),
            '"' if double_quoted => out.push('"'),
            '\\' => out.push('\\'),
            'n' if double_quoted => out.push('\n'),
            'r' if double_quoted => out.push('\r'),
            't' if double_quoted => out.push('\t'),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

fn resolve_wasm_stub(specifier: &str, file_path: &Path, modules_root: &Path) -> Result<PathBuf, String> {
    let current_dir = file_path
        .parent()
        .ok_or_else(|| "wasm import requires a parent directory".to_string())?;
    let base_dir = if specifier.starts_with('.') {
        current_dir
    } else {
        modules_root
    };

    let root_path = normalize_path(base_dir.join(specifier));
    if !root_path.starts_with(modules_root) {
        return Err(format!(
            "wasm import must resolve inside php_modules/ ({}).",
            specifier
        ));
    }

    let manifest_path = root_path.join("deka.json");
    if !manifest_path.is_file() {
        return Err(format!(
            "Missing wasm module manifest for '{}' (expected {}).",
            specifier,
            manifest_path.display()
        ));
    }

    let stub_spec = read_stub_path(&manifest_path)?;
    let stub_path = match stub_spec {
        Some(path) => {
            let stub_path = PathBuf::from(path);
            if stub_path.is_absolute() {
                stub_path
            } else {
                root_path.join(stub_path)
            }
        }
        None => root_path.join("module.d.phpx"),
    };

    if !stub_path.is_file() {
        return Err(format!(
            "Missing wasm type stubs for '{}' (expected {}).",
            specifier,
            stub_path.display()
        ));
    }

    Ok(stub_path)
}

fn read_stub_path(manifest_path: &Path) -> Result<Option<String>, String> {
    let raw = fs::read_to_string(manifest_path).map_err(|err| {
        format!(
            "Failed to read wasm manifest {}: {}",
            manifest_path.display(),
            err
        )
    })?;
    let json: serde_json::Value = serde_json::from_str(&raw).map_err(|err| {
        format!(
            "Failed to parse wasm manifest {}: {}",
            manifest_path.display(),
            err
        )
    })?;
    let stubs = json
        .get("stubs")
        .and_then(|val| val.as_str())
        .or_else(|| json.get("stub").and_then(|val| val.as_str()));
    Ok(stubs.map(|value| value.to_string()))
}

fn find_modules_root(file_path: &Path) -> Option<PathBuf> {
    let mut dir = file_path.parent()?;
    loop {
        if dir.file_name().and_then(|name| name.to_str()) == Some("php_modules") {
            return Some(dir.to_path_buf());
        }
        let candidate = dir.join("php_modules");
        if candidate.is_dir() {
            return Some(candidate);
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => break,
        }
    }
    None
}

fn normalize_path(path: PathBuf) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::RootDir | Component::Prefix(_) => out.push(component.as_os_str()),
            Component::Normal(_) => out.push(component.as_os_str()),
        }
    }
    out
}

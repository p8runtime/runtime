use crate::parser::ast::{BinaryOp, Expr, ObjectKey};
use crate::phpx::typeck::types::{ObjectField, PrimitiveType, Type, merge_types};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct StructInfo {
    pub fields: BTreeMap<String, Type>,
    pub embeds: Vec<String>,
    pub defaults: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct EnumParamInfo {
    pub name: String,
    pub ty: Option<Type>,
}

#[derive(Debug, Clone)]
pub struct EnumCaseInfo {
    pub params: Vec<EnumParamInfo>,
}

#[derive(Debug, Clone)]
pub struct EnumInfo {
    pub cases: BTreeMap<String, EnumCaseInfo>,
    pub backed: Option<PrimitiveType>,
    pub type_params: Vec<String>,
}

pub struct InferContext<'a> {
    pub source: &'a [u8],
    pub vars: &'a HashMap<String, Type>,
    pub structs: &'a HashMap<String, StructInfo>,
    pub interfaces: &'a HashMap<String, BTreeMap<String, ObjectField>>,
    pub functions: &'a HashMap<String, Type>,
    pub enums: &'a HashMap<String, EnumInfo>,
}

fn resolve_struct_field_type(name: &str, field: &str, ctx: &InferContext) -> Option<Type> {
    let mut visited = HashSet::new();
    let (ty, ambiguous) = resolve_struct_field_type_inner(name, field, ctx, &mut visited);
    if ambiguous { None } else { ty }
}

fn resolve_struct_field_type_inner(
    name: &str,
    field: &str,
    ctx: &InferContext,
    visited: &mut HashSet<String>,
) -> (Option<Type>, bool) {
    if !visited.insert(name.to_string()) {
        return (None, false);
    }
    let Some(info) = ctx.structs.get(name) else {
        return (None, false);
    };

    if let Some(ty) = info.fields.get(field) {
        return (Some(ty.clone()), false);
    }

    let mut found: Option<Type> = None;
    let mut ambiguous = false;

    for embed in &info.embeds {
        let (ty, is_ambiguous) = resolve_struct_field_type_inner(embed, field, ctx, visited);
        if is_ambiguous {
            ambiguous = true;
        }
        if let Some(ty) = ty {
            if found.is_some() {
                ambiguous = true;
            } else {
                found = Some(ty);
            }
        }
    }

    (found, ambiguous)
}

pub fn infer_expr(expr: &Expr, ctx: &InferContext) -> Type {
    if let Some(literal) = literal_type(expr) {
        return literal;
    }
    match expr {
        Expr::Variable { span, .. } => {
            let name = token_text(ctx.source, *span);
            let name = name.strip_prefix('$').unwrap_or(&name);
            ctx.vars.get(name).cloned().unwrap_or(Type::Unknown)
        }
        Expr::Array { items, .. } => {
            let mut element_ty = Type::Unknown;
            for item in *items {
                let value_ty = infer_expr(&item.value, ctx);
                if item.unpack {
                    match value_ty {
                        Type::Applied { base, args } if base.eq_ignore_ascii_case("array") => {
                            let inner = args.get(0).cloned().unwrap_or(Type::Unknown);
                            element_ty = merge_types(&element_ty, &inner);
                        }
                        Type::Array => {
                            element_ty = merge_types(&element_ty, &Type::Unknown);
                        }
                        _ => {
                            element_ty = merge_types(&element_ty, &Type::Unknown);
                        }
                    }
                } else {
                    element_ty = merge_types(&element_ty, &value_ty);
                }
            }
            Type::Applied {
                base: "array".to_string(),
                args: vec![element_ty],
            }
        }
        Expr::ObjectLiteral { items, .. } => {
            let mut fields = BTreeMap::new();
            for item in *items {
                if let Expr::Spread { expr, .. } = item.value {
                    let spread_ty = infer_expr(expr, ctx);
                    if let Some(spread_fields) = object_type_fields(&spread_ty, ctx) {
                        for (name, field) in spread_fields {
                            fields.insert(name, field);
                        }
                    }
                    continue;
                }
                let key = object_key_name(item.key, ctx.source);
                let value_ty = infer_expr(&item.value, ctx);
                fields.insert(
                    key,
                    ObjectField {
                        ty: value_ty,
                        optional: false,
                        is_mut: false,
                    },
                );
            }
            Type::ObjectShape(fields)
        }
        Expr::StructLiteral { name, .. } => {
            let raw = token_text(ctx.source, name.span);
            let struct_name = raw.trim_start_matches('\\').to_string();
            if ctx.structs.contains_key(&struct_name) {
                Type::Struct(struct_name)
            } else {
                Type::Unknown
            }
        }
        Expr::JsxElement { .. } | Expr::JsxFragment { .. } => Type::Unknown,
        Expr::DotAccess {
            target, property, ..
        } => {
            // DekaScript enum variant access: `Status.Ready` is syntactic sugar
            // for `Status::Ready` when the target names an enum and the property
            // names one of its cases.
            let prop_name = token_text(ctx.source, property.span);
            if let Some(target_name) = extract_ident(target, ctx.source) {
                if let Some(info) = ctx.enums.get(&target_name) {
                    if info.cases.contains_key(&prop_name) {
                        return Type::EnumCase {
                            enum_name: target_name,
                            case_name: prop_name,
                            args: Vec::new(),
                        };
                    }
                }
            }
            let target_ty = infer_expr(target, ctx);
            match target_ty {
                Type::ObjectShape(fields) => fields
                    .get(&prop_name)
                    .map(|field| {
                        if field.optional {
                            Type::Applied {
                                base: "Option".to_string(),
                                args: vec![field.ty.clone()],
                            }
                        } else {
                            field.ty.clone()
                        }
                    })
                    .unwrap_or(Type::Unknown),
                Type::Struct(name) => {
                    resolve_struct_field_type(&name, &prop_name, ctx).unwrap_or(Type::Unknown)
                }
                Type::Interface(name) => ctx
                    .interfaces
                    .get(&name)
                    .and_then(|fields| fields.get(&prop_name))
                    .map(|field| {
                        if field.optional {
                            Type::Applied {
                                base: "Option".to_string(),
                                args: vec![field.ty.clone()],
                            }
                        } else {
                            field.ty.clone()
                        }
                    })
                    .unwrap_or(Type::Unknown),
                Type::Enum(name) => {
                    if name.eq_ignore_ascii_case("Option") || name.eq_ignore_ascii_case("Result") {
                        if prop_name == "name" {
                            Type::Primitive(PrimitiveType::String)
                        } else {
                            Type::Unknown
                        }
                    } else {
                        infer_enum_field(name, &prop_name, ctx)
                    }
                }
                Type::EnumCase {
                    enum_name,
                    case_name,
                    args,
                } => {
                    if enum_name.eq_ignore_ascii_case("Option") {
                        if case_name.eq_ignore_ascii_case("Some") && prop_name == "value" {
                            args.get(0).cloned().unwrap_or(Type::Unknown)
                        } else if prop_name == "name" {
                            Type::Primitive(PrimitiveType::String)
                        } else {
                            Type::Unknown
                        }
                    } else if enum_name.eq_ignore_ascii_case("Result") {
                        if case_name.eq_ignore_ascii_case("Ok") && prop_name == "value" {
                            args.get(0).cloned().unwrap_or(Type::Unknown)
                        } else if case_name.eq_ignore_ascii_case("Err") && prop_name == "error" {
                            args.get(1).cloned().unwrap_or(Type::Unknown)
                        } else if prop_name == "name" {
                            Type::Primitive(PrimitiveType::String)
                        } else {
                            Type::Unknown
                        }
                    } else {
                        infer_enum_case_field(&enum_name, &case_name, &prop_name, ctx)
                    }
                }
                Type::Applied { base, args: _ } => {
                    if base.eq_ignore_ascii_case("Option") || base.eq_ignore_ascii_case("Result") {
                        if prop_name == "name" {
                            Type::Primitive(PrimitiveType::String)
                        } else {
                            Type::Unknown
                        }
                    } else {
                        Type::Unknown
                    }
                }
                _ => Type::Unknown,
            }
        }
        Expr::Call { func, .. } => {
            if let Expr::Variable { span, .. } = &**func {
                let name = token_text(ctx.source, *span);
                if !name.starts_with('$') {
                    if let Some(ret) = ctx.functions.get(&name) {
                        return ret.clone();
                    }
                }
            }
            Type::Unknown
        }
        Expr::Closure {
            params,
            return_type,
            ..
        } => {
            let param_types = params
                .iter()
                .map(|param| {
                    param
                        .ty
                        .map(|ty| resolve_ast_type(ctx, ty))
                        .unwrap_or(Type::Unknown)
                })
                .collect();
            let return_ty = return_type
                .map(|ty| resolve_ast_type(ctx, ty))
                .unwrap_or(Type::Unknown);
            Type::Function {
                params: param_types,
                return_type: Box::new(return_ty),
            }
        }
        Expr::Await { expr, .. } => {
            let awaited = infer_expr(expr, ctx);
            match awaited {
                Type::Applied { base, args } if base.eq_ignore_ascii_case("Promise") => {
                    args.first().cloned().unwrap_or(Type::Unknown)
                }
                _ => Type::Unknown,
            }
        }
        Expr::Binary {
            op, left, right, ..
        } => {
            let left_ty = infer_expr(left, ctx);
            let right_ty = infer_expr(right, ctx);
            if *op == BinaryOp::Coalesce {
                return merge_types(&left_ty, &right_ty);
            }
            if *op == BinaryOp::Pipe {
                return infer_pipe_call(right, ctx);
            }
            infer_binary_op(*op, &left_ty, &right_ty)
        }
        Expr::Ternary {
            condition,
            if_true,
            if_false,
            ..
        } => {
            let true_ty = if let Some(expr) = if_true {
                infer_expr(expr, ctx)
            } else {
                infer_expr(condition, ctx)
            };
            let false_ty = infer_expr(if_false, ctx);
            merge_types(&true_ty, &false_ty)
        }
        Expr::Match { arms, .. } => {
            let mut out = Type::Unknown;
            for arm in *arms {
                let body_ty = infer_expr(arm.body, ctx);
                out = merge_types(&out, &body_ty);
            }
            out
        }
        Expr::Unsafe { body, .. } => infer_expr(body, ctx),
        Expr::Assign { expr: rhs, .. } | Expr::AssignRef { expr: rhs, .. } => infer_expr(rhs, ctx),
        Expr::New { .. } => Type::Unknown,
        Expr::ClassConstFetch {
            class, constant, ..
        } => {
            if let (Some(class_name), Some(case_name)) = (
                extract_ident(class, ctx.source),
                extract_ident(constant, ctx.source),
            ) {
                if class_name.eq_ignore_ascii_case("Option")
                    || class_name.eq_ignore_ascii_case("Result")
                {
                    if matches!(case_name.as_str(), "Some" | "None" | "Ok" | "Err") {
                        return Type::EnumCase {
                            enum_name: if class_name.eq_ignore_ascii_case("Option") {
                                "Option".to_string()
                            } else {
                                "Result".to_string()
                            },
                            case_name,
                            args: Vec::new(),
                        };
                    }
                }
                if let Some(info) = ctx.enums.get(&class_name) {
                    if info.cases.contains_key(&case_name) {
                        return Type::Enum(class_name);
                    }
                }
            }
            Type::Unknown
        }
        Expr::StaticCall { class, method, .. } => {
            if let (Some(class_name), Some(case_name)) = (
                extract_ident(class, ctx.source),
                extract_ident(method, ctx.source),
            ) {
                if class_name.eq_ignore_ascii_case("Option") {
                    if case_name.eq_ignore_ascii_case("Some") {
                        let arg_ty = match &expr {
                            Expr::StaticCall { args, .. } if !args.is_empty() => {
                                infer_expr(&args[0].value, ctx)
                            }
                            _ => Type::Unknown,
                        };
                        return Type::EnumCase {
                            enum_name: "Option".to_string(),
                            case_name,
                            args: vec![arg_ty],
                        };
                    }
                    if case_name.eq_ignore_ascii_case("None") {
                        return Type::EnumCase {
                            enum_name: "Option".to_string(),
                            case_name,
                            args: Vec::new(),
                        };
                    }
                }
                if class_name.eq_ignore_ascii_case("Result") {
                    if case_name.eq_ignore_ascii_case("Ok") {
                        let ok_ty = match &expr {
                            Expr::StaticCall { args, .. } if !args.is_empty() => {
                                infer_expr(&args[0].value, ctx)
                            }
                            _ => Type::Unknown,
                        };
                        return Type::EnumCase {
                            enum_name: "Result".to_string(),
                            case_name,
                            args: vec![ok_ty, Type::Unknown],
                        };
                    }
                    if case_name.eq_ignore_ascii_case("Err") {
                        let err_ty = match &expr {
                            Expr::StaticCall { args, .. } if !args.is_empty() => {
                                infer_expr(&args[0].value, ctx)
                            }
                            _ => Type::Unknown,
                        };
                        return Type::EnumCase {
                            enum_name: "Result".to_string(),
                            case_name,
                            args: vec![Type::Unknown, err_ty],
                        };
                    }
                }
                if let Some(info) = ctx.enums.get(&class_name) {
                    if info.cases.contains_key(&case_name) {
                        return Type::EnumCase {
                            enum_name: class_name,
                            case_name,
                            args: Vec::new(),
                        };
                    }
                }
            }
            Type::Unknown
        }
        _ => Type::Unknown,
    }
}

fn infer_binary_op(op: BinaryOp, left: &Type, right: &Type) -> Type {
    let is_int = |t: &Type| matches!(t, Type::Primitive(PrimitiveType::Int));
    let is_float = |t: &Type| matches!(t, Type::Primitive(PrimitiveType::Float));
    let is_bool = |t: &Type| matches!(t, Type::Primitive(PrimitiveType::Bool));
    let is_string = |t: &Type| matches!(t, Type::Primitive(PrimitiveType::String));
    let is_unknown = |t: &Type| matches!(t, Type::Unknown);

    match op {
        BinaryOp::Plus | BinaryOp::Minus | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod | BinaryOp::Pow => {
            if is_unknown(left) || is_unknown(right) {
                Type::Unknown
            } else if is_int(left) && is_int(right) {
                Type::Primitive(PrimitiveType::Int)
            } else if (is_int(left) || is_float(left)) && (is_int(right) || is_float(right)) {
                Type::Primitive(PrimitiveType::Float)
            } else {
                Type::Unknown
            }
        }
        BinaryOp::Concat => {
            if is_unknown(left) && is_unknown(right) {
                Type::Unknown
            } else if is_string(left) || is_string(right) || is_unknown(left) || is_unknown(right) {
                Type::Primitive(PrimitiveType::String)
            } else {
                Type::Unknown
            }
        }
        BinaryOp::Eq
        | BinaryOp::EqEq
        | BinaryOp::EqEqEq
        | BinaryOp::NotEq
        | BinaryOp::NotEqEq
        | BinaryOp::Lt
        | BinaryOp::LtEq
        | BinaryOp::Gt
        | BinaryOp::GtEq
        | BinaryOp::Spaceship
        | BinaryOp::Instanceof => Type::Primitive(PrimitiveType::Bool),
        BinaryOp::And
        | BinaryOp::Or
        | BinaryOp::BitAnd
        | BinaryOp::BitOr
        | BinaryOp::BitXor
        | BinaryOp::LogicalAnd
        | BinaryOp::LogicalOr
        | BinaryOp::LogicalXor => {
            if is_unknown(left) || is_unknown(right) {
                Type::Unknown
            } else if is_bool(left) && is_bool(right) {
                Type::Primitive(PrimitiveType::Bool)
            } else {
                Type::Primitive(PrimitiveType::Bool)
            }
        }
        BinaryOp::ShiftLeft | BinaryOp::ShiftRight => {
            if is_int(left) && is_int(right) {
                Type::Primitive(PrimitiveType::Int)
            } else {
                Type::Unknown
            }
        }
        BinaryOp::Coalesce => merge_types(left, right),
        BinaryOp::Pipe => Type::Unknown,
    }
}

fn infer_pipe_call(right: &Expr, ctx: &InferContext) -> Type {
    // `a |> f(b, c)` desugars to `f(a, b, c)`. The RHS may be a call whose
    // callee is the function, or just a bare function reference.
    let func_expr = match right {
        Expr::Call { func, .. } => func,
        _ => right,
    };
    if let Expr::Variable { span, .. } = func_expr {
        let name = token_text(ctx.source, *span);
        if !name.starts_with('$') {
            if let Some(ret) = ctx.functions.get(&name) {
                return ret.clone();
            }
        }
    }
    if let Expr::Closure { return_type, .. } = func_expr {
        return return_type
            .map(|ty| resolve_ast_type(ctx, ty))
            .unwrap_or(Type::Unknown);
    }
    Type::Unknown
}

pub fn literal_type(expr: &Expr) -> Option<Type> {
    match expr {
        Expr::Integer { .. } => Some(Type::Primitive(PrimitiveType::Int)),
        Expr::Float { .. } => Some(Type::Primitive(PrimitiveType::Float)),
        Expr::Boolean { .. } => Some(Type::Primitive(PrimitiveType::Bool)),
        Expr::String { .. } => Some(Type::Primitive(PrimitiveType::String)),
        Expr::Null { .. } => Some(Type::Primitive(PrimitiveType::Null)),
        _ => None,
    }
}

fn object_type_fields(
    ty: &Type,
    ctx: &InferContext,
) -> Option<std::collections::BTreeMap<String, ObjectField>> {
    match ty {
        Type::ObjectShape(fields) => Some(fields.clone()),
        Type::Interface(name) => ctx.interfaces.get(name).cloned(),
        Type::Struct(name) => ctx.structs.get(name).map(|info| {
            info.fields
                .iter()
                .map(|(name, ty)| {
                    (
                        name.clone(),
                        ObjectField {
                            ty: ty.clone(),
                            optional: false,
                            is_mut: false,
                        },
                    )
                })
                .collect()
        }),
        Type::Applied { base, args } if base.eq_ignore_ascii_case("Object") => {
            args.first().and_then(|arg| object_type_fields(arg, ctx))
        }
        _ => None,
    }
}

fn token_text(source: &[u8], span: crate::parser::span::Span) -> String {
    let start = span.start;
    let end = span.end.min(source.len());
    String::from_utf8_lossy(&source[start..end]).to_string()
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

fn extract_ident(expr: &Expr, source: &[u8]) -> Option<String> {
    match expr {
        Expr::Variable { span, .. } => {
            let name = token_text(source, *span);
            if name.starts_with('$') {
                None
            } else {
                Some(name)
            }
        }
        _ => None,
    }
}

fn infer_enum_field(enum_name: String, field: &str, ctx: &InferContext) -> Type {
    let Some(info) = ctx.enums.get(&enum_name) else {
        return Type::Unknown;
    };
    if field == "name" {
        return Type::Primitive(PrimitiveType::String);
    }
    if field == "value" {
        return match info.backed {
            Some(PrimitiveType::Int) => Type::Primitive(PrimitiveType::Int),
            Some(PrimitiveType::String) => Type::Primitive(PrimitiveType::String),
            _ => Type::Unknown,
        };
    }

    let mut merged: Option<Type> = None;
    for case in info.cases.values() {
        let Some(param) = case.params.iter().find(|param| param.name == field) else {
            return Type::Unknown;
        };
        let param_ty = param.ty.clone().unwrap_or(Type::Unknown);
        merged = Some(match merged {
            Some(existing) => merge_types(&existing, &param_ty),
            None => param_ty,
        });
    }
    merged.unwrap_or(Type::Unknown)
}

fn infer_enum_case_field(
    enum_name: &str,
    case_name: &str,
    field: &str,
    ctx: &InferContext,
) -> Type {
    let Some(info) = ctx.enums.get(enum_name) else {
        return Type::Unknown;
    };
    if field == "name" {
        return Type::Primitive(PrimitiveType::String);
    }
    if field == "value" {
        return match info.backed {
            Some(PrimitiveType::Int) => Type::Primitive(PrimitiveType::Int),
            Some(PrimitiveType::String) => Type::Primitive(PrimitiveType::String),
            _ => Type::Unknown,
        };
    }
    let Some(case) = info.cases.get(case_name) else {
        return Type::Unknown;
    };
    let Some(param) = case.params.iter().find(|param| param.name == field) else {
        return Type::Unknown;
    };
    param.ty.clone().unwrap_or(Type::Unknown)
}

fn resolve_ast_type(ctx: &InferContext, ty: &crate::parser::ast::Type) -> Type {
    use crate::parser::ast::Type as AstType;
    use crate::parser::lexer::token::TokenKind;
    match ty {
        AstType::Simple(token) => match token.kind {
            TokenKind::TypeInt => Type::Primitive(PrimitiveType::Int),
            TokenKind::TypeString => Type::Primitive(PrimitiveType::String),
            TokenKind::TypeBool => Type::Primitive(PrimitiveType::Bool),
            TokenKind::TypeFloat => Type::Primitive(PrimitiveType::Float),
            TokenKind::TypeBytes => Type::Primitive(PrimitiveType::Bytes),
            TokenKind::TypeNull => Type::Primitive(PrimitiveType::Null),
            _ => Type::Unknown,
        },
        AstType::Name(name) => {
            let text = name
                .parts
                .iter()
                .map(|part| token_text(ctx.source, part.span))
                .collect::<Vec<_>>()
                .join("\\");
            match text.to_ascii_lowercase().as_str() {
                "int" | "integer" | "number" => Type::Primitive(PrimitiveType::Int),
                "float" | "double" => Type::Primitive(PrimitiveType::Float),
                "bool" | "boolean" => Type::Primitive(PrimitiveType::Bool),
                "string" => Type::Primitive(PrimitiveType::String),
                "bytes" => Type::Primitive(PrimitiveType::Bytes),
                "null" => Type::Primitive(PrimitiveType::Null),
                _ => Type::Unknown,
            }
        }
        AstType::Applied { base, args } => {
            if let AstType::Simple(token) = base {
                let name = token_text(ctx.source, token.span).to_ascii_lowercase();
                if name == "array" && args.len() == 1 {
                    return Type::Applied {
                        base: "array".to_string(),
                        args: vec![resolve_ast_type(ctx, &args[0])],
                    };
                }
                if name == "option" && args.len() == 1 {
                    return Type::Applied {
                        base: "Option".to_string(),
                        args: vec![resolve_ast_type(ctx, &args[0])],
                    };
                }
            }
            Type::Unknown
        }
        _ => Type::Unknown,
    }
}

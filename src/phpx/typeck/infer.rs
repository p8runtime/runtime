use crate::parser::ast::{BinaryOp, Expr, ObjectKey};
use crate::phpx::typeck::types::{merge_types, ObjectField, PrimitiveType, Type};
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
    pub required: bool,
    pub variadic: bool,
}

#[derive(Debug, Clone)]
pub struct EnumCaseInfo {
    pub params: Vec<EnumParamInfo>,
}

#[derive(Debug, Clone)]
pub struct EnumInfo {
    pub cases: BTreeMap<String, EnumCaseInfo>,
    pub backed: Option<PrimitiveType>,
}

pub struct InferContext<'a> {
    pub source: &'a [u8],
    pub vars: &'a HashMap<String, Type>,
    pub structs: &'a HashMap<String, StructInfo>,
    pub functions: &'a HashMap<String, Type>,
    pub enums: &'a HashMap<String, EnumInfo>,
}

fn resolve_struct_field_type(name: &str, field: &str, ctx: &InferContext) -> Option<Type> {
    let mut visited = HashSet::new();
    let (ty, ambiguous) = resolve_struct_field_type_inner(name, field, ctx, &mut visited);
    if ambiguous {
        None
    } else {
        ty
    }
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
            ctx.vars
                .get(name)
                .cloned()
                .unwrap_or(Type::Unknown)
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
                let key = object_key_name(item.key, ctx.source);
                let value_ty = infer_expr(&item.value, ctx);
                fields.insert(
                    key,
                    ObjectField {
                        ty: value_ty,
                        optional: false,
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
        Expr::DotAccess { target, property, .. } => {
            let target_ty = infer_expr(target, ctx);
            let prop_name = token_text(ctx.source, property.span);
            match target_ty {
                Type::ObjectShape(fields) => fields
                    .get(&prop_name)
                    .map(|field| {
                        if field.optional {
                            merge_types(&field.ty, &Type::Primitive(PrimitiveType::Null))
                        } else {
                            field.ty.clone()
                        }
                    })
                    .unwrap_or(Type::Unknown),
                Type::Struct(name) => resolve_struct_field_type(&name, &prop_name, ctx)
                    .unwrap_or(Type::Unknown),
                Type::Enum(name) => {
                    if name.eq_ignore_ascii_case("Option")
                        || name.eq_ignore_ascii_case("Result")
                    {
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
                    if base.eq_ignore_ascii_case("Option") || base.eq_ignore_ascii_case("Result")
                    {
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
        Expr::Binary { op, left, right, .. } => {
            if *op == BinaryOp::Coalesce {
                let left_ty = infer_expr(left, ctx);
                let right_ty = infer_expr(right, ctx);
                return merge_types(&left_ty, &right_ty);
            }
            Type::Unknown
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
        Expr::Assign { expr: rhs, .. } | Expr::AssignRef { expr: rhs, .. } => {
            infer_expr(rhs, ctx)
        }
        Expr::New { .. } => Type::Unknown,
        Expr::ClassConstFetch { class, constant, .. } => {
            if let (Some(class_name), Some(case_name)) =
                (extract_ident(class, ctx.source), extract_ident(constant, ctx.source))
            {
                if class_name.eq_ignore_ascii_case("Option")
                    || class_name.eq_ignore_ascii_case("Result")
                {
                    if matches!(
                        case_name.as_str(),
                        "Some" | "None" | "Ok" | "Err"
                    ) {
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
        Expr::StaticCall {
            class,
            method,
            ..
        } => {
            if let (Some(class_name), Some(case_name)) =
                (extract_ident(class, ctx.source), extract_ident(method, ctx.source))
            {
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

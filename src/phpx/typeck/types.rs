use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrimitiveType {
    Int,
    Float,
    Bool,
    String,
    Null,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Unknown,
    Mixed,
    Primitive(PrimitiveType),
    Array,
    Object,
    VNode,
    ObjectShape(BTreeMap<String, ObjectField>),
    Struct(String),
    Interface(String),
    Enum(String),
    EnumCase {
        enum_name: String,
        case_name: String,
        args: Vec<Type>,
    },
    Union(Vec<Type>),
    TypeParam(String),
    Applied { base: String, args: Vec<Type> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectField {
    pub ty: Type,
    pub optional: bool,
}

impl Type {
    pub fn name(&self) -> String {
        match self {
            Type::Unknown => "unknown".to_string(),
            Type::Mixed => "mixed".to_string(),
            Type::Primitive(prim) => match prim {
                PrimitiveType::Int => "int".to_string(),
                PrimitiveType::Float => "float".to_string(),
                PrimitiveType::Bool => "bool".to_string(),
                PrimitiveType::String => "string".to_string(),
                PrimitiveType::Null => "null".to_string(),
            },
            Type::Array => "array".to_string(),
            Type::Object => "object".to_string(),
            Type::VNode => "VNode".to_string(),
            Type::ObjectShape(fields) => {
                let mut out = String::from("Object<{");
                let mut first = true;
                for (name, field) in fields.iter() {
                    if !first {
                        out.push_str(", ");
                    }
                    first = false;
                    out.push_str(name);
                    if field.optional {
                        out.push('?');
                    }
                    out.push_str(": ");
                    out.push_str(&field.ty.name());
                }
                out.push_str("}>");
                out
            }
            Type::Struct(name) => format!("struct {name}"),
            Type::Interface(name) => format!("interface {name}"),
            Type::Enum(name) => format!("enum {name}"),
            Type::EnumCase {
                enum_name,
                case_name,
                args,
            } => {
                if args.is_empty() {
                    format!("enum {enum_name}::{case_name}")
                } else {
                    let rendered = args.iter().map(|t| t.name()).collect::<Vec<_>>();
                    format!("enum {enum_name}<{}>::{case_name}", rendered.join(", "))
                }
            }
            Type::Union(types) => {
                let parts = types.iter().map(|t| t.name()).collect::<Vec<_>>();
                parts.join(" | ")
            }
            Type::TypeParam(name) => name.clone(),
            Type::Applied { base, args } => {
                let rendered = args.iter().map(|t| t.name()).collect::<Vec<_>>();
                format!("{base}<{}>", rendered.join(", "))
            }
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

pub fn merge_types(left: &Type, right: &Type) -> Type {
    if left == right {
        return left.clone();
    }
    if matches!(left, Type::Unknown) {
        return right.clone();
    }
    if matches!(right, Type::Unknown) {
        return left.clone();
    }
    if matches!(left, Type::Mixed) || matches!(right, Type::Mixed) {
        return Type::Mixed;
    }
    match (left, right) {
        (Type::Primitive(PrimitiveType::Int), Type::Primitive(PrimitiveType::Float))
        | (Type::Primitive(PrimitiveType::Float), Type::Primitive(PrimitiveType::Int)) => {
            return Type::Primitive(PrimitiveType::Float);
        }
        _ => {}
    }
    let mut out = Vec::new();
    collect_union_types(left, &mut out);
    collect_union_types(right, &mut out);
    dedupe_types(&mut out);
    if out.len() == 1 {
        out.remove(0)
    } else {
        Type::Union(out)
    }
}

fn collect_union_types(ty: &Type, out: &mut Vec<Type>) {
    match ty {
        Type::Union(types) => {
            for inner in types.iter() {
                collect_union_types(inner, out);
            }
        }
        _ => out.push(ty.clone()),
    }
}

fn dedupe_types(types: &mut Vec<Type>) {
    let mut idx = 0;
    while idx < types.len() {
        let mut dup = false;
        for j in 0..idx {
            if types[j] == types[idx] {
                dup = true;
                break;
            }
        }
        if dup {
            types.remove(idx);
        } else {
            idx += 1;
        }
    }
}

use super::*;

#[derive(Debug, Clone, PartialEq)]
pub(in crate::phpx::typeck::check) struct ParamSig {
    pub(in crate::phpx::typeck::check) ty: Option<Type>,
    pub(in crate::phpx::typeck::check) required: bool,
}

#[derive(Debug, Clone)]
pub(in crate::phpx::typeck::check) struct TypeParamSig {
    pub(in crate::phpx::typeck::check) name: String,
    pub(in crate::phpx::typeck::check) constraint: Option<Type>,
}

#[derive(Debug, Clone)]
pub(in crate::phpx::typeck::check) struct FunctionSig {
    pub(in crate::phpx::typeck::check) type_params: Vec<TypeParamSig>,
    pub(in crate::phpx::typeck::check) params: Vec<ParamSig>,
    pub(in crate::phpx::typeck::check) return_type: Option<Type>,
    pub(in crate::phpx::typeck::check) variadic: bool,
}

#[derive(Debug, Clone)]
pub struct ExternalTypeParamSig {
    pub name: String,
    pub constraint: Option<Type>,
}

#[derive(Debug, Clone)]
pub struct ExternalParamSig {
    pub ty: Option<Type>,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct ExternalFunctionSig {
    pub type_params: Vec<ExternalTypeParamSig>,
    pub params: Vec<ExternalParamSig>,
    pub return_type: Option<Type>,
    pub variadic: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(in crate::phpx::typeck::check) struct MethodSig {
    pub(in crate::phpx::typeck::check) params: Vec<ParamSig>,
    pub(in crate::phpx::typeck::check) return_type: Option<Type>,
    pub(in crate::phpx::typeck::check) variadic: bool,
    pub(in crate::phpx::typeck::check) mutable: bool,
}

#[derive(Debug, Clone)]
pub(in crate::phpx::typeck::check) struct InterfaceInfo {
    pub(in crate::phpx::typeck::check) methods: HashMap<String, MethodSig>,
    pub(in crate::phpx::typeck::check) fields: BTreeMap<String, ObjectField>,
}

#[derive(Debug, Clone)]
pub(in crate::phpx::typeck::check) struct TypeAliasInfo {
    pub(in crate::phpx::typeck::check) params: Vec<TypeParamSig>,
    pub(in crate::phpx::typeck::check) ty: Type,
    pub(in crate::phpx::typeck::check) span: Span,
}

#[derive(Debug, Clone)]
pub struct TypeckProgramSummary {
    pub structs: HashMap<String, StructInfo>,
    pub enums: HashMap<String, EnumInfo>,
    pub functions: HashMap<String, TypeckFunctionInfo>,
    pub type_aliases: HashMap<String, Type>,
}

#[derive(Debug, Clone)]
pub struct TypeckFunctionInfo {
    pub params: Vec<TypeckParamInfo>,
    pub return_type: Option<Type>,
    pub variadic: bool,
}

#[derive(Debug, Clone)]
pub struct TypeckParamInfo {
    pub ty: Option<Type>,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub(in crate::phpx::typeck::check) enum StructFieldResolution {
    Found(Type),
    Missing,
    Ambiguous,
}

#[derive(Debug, Clone)]
pub struct TypeError {
    pub span: Span,
    pub message: String,
    /// Defaults to `Severity::Error` at every existing call site. Only the
    /// deka#59 proof-of-concept warning (see `check/poc_warning.rs`) sets
    /// `Severity::Warning` today.
    pub severity: Severity,
}

impl TypeError {
    pub fn is_error(&self) -> bool {
        self.severity.is_error()
    }
}

pub(in crate::phpx::typeck::check) struct JsxExprValidator {
    pub(in crate::phpx::typeck::check) errors: Vec<TypeError>,
}



use crate::parser::ast::visitor::{Visitor, walk_expr};
use crate::parser::ast::{
    BinaryOp, ClassKind, ClassMember, Expr, ExprId, JsxChild, Name, ObjectKey, Program,
    PropertyEntry, Stmt, StmtId, Type as AstType, TypeParam, UnaryOp,
};
use crate::parser::lexer::token::TokenKind;
use crate::parser::span::Span;
use crate::phpx::typeck::infer::{
    EnumCaseInfo, EnumInfo, EnumParamInfo, InferContext, StructInfo, infer_expr,
};
use crate::phpx::typeck::types::{ObjectField, PrimitiveType, Type, merge_types};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

mod api;
mod assignability;
mod collect;
mod context;
mod enums_match;
mod expressions;
mod helpers;
mod jsx;
mod methods;
mod model;
mod statements;
mod structs;
mod type_resolution;

pub use api::{
    check_program, check_program_with_path, check_program_with_path_and_externals,
    external_functions_from_stub, format_type_errors, summarize_program_with_path,
};
pub use model::{
    ExternalFunctionSig, ExternalParamSig, ExternalTypeParamSig, TypeError, TypeckFunctionInfo,
    TypeckParamInfo, TypeckProgramSummary,
};

pub(in crate::phpx::typeck::check) use context::CheckContext;
pub(in crate::phpx::typeck::check) use helpers::*;
pub(in crate::phpx::typeck::check) use model::{
    FunctionSig, ImplRecord, InterfaceInfo, JsxExprValidator, MethodSig, ParamSig,
    SelfFieldValidator, StructFieldResolution, TraitInfo, TypeAliasInfo, TypeParamSig,
};

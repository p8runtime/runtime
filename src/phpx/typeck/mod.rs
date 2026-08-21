mod check;
mod infer;
mod types;

pub use check::{
    ExternalFunctionSig, ExternalParamSig, ExternalTypeParamSig, TypeError, TypeckFunctionInfo,
    TypeckParamInfo, TypeckProgramSummary, check_program, check_program_with_imports,
    check_program_with_path, check_program_with_path_and_externals, external_functions_from_stub,
    format_type_errors, summarize_program_with_path,
};
pub use infer::{EnumCaseInfo, EnumInfo, EnumParamInfo, StructInfo};
pub use types::{PrimitiveType, Type};

#[cfg(test)]
mod tests;

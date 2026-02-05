mod check;
mod infer;
mod types;

pub use check::{
    check_program,
    check_program_with_path,
    check_program_with_path_and_externals,
    external_functions_from_stub,
    format_type_errors,
    ExternalFunctionSig,
    ExternalParamSig,
    ExternalTypeParamSig,
    TypeError,
};
pub use types::{PrimitiveType, Type};

#[cfg(test)]
mod tests;

pub mod builtins;
pub mod compiler;
pub mod core;
pub mod parser;
pub mod phpx;
pub mod runtime;
pub mod sapi;
pub mod vm;

#[cfg(target_arch = "wasm32")]
pub mod wasm_exports;

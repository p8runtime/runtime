#![forbid(unsafe_code)] // memory safety is the point of this runtime; see SECURITY.md
pub mod builtins;
pub mod compiler;
pub mod core;
pub mod fcgi;
pub mod parser;
pub mod phpt;
pub mod runtime;
pub mod sapi;
pub mod v2;
pub mod vm;

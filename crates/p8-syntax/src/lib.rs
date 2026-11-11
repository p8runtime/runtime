#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's front end: PHP 8.4 source to an AST (lexer, parser, spans), and the
//! values of literal tokens, for p8's compiler.

pub mod ast;
pub mod lexer;
pub mod literal;
pub mod line_index;
pub mod parser;
pub mod span;

pub use span::Span;

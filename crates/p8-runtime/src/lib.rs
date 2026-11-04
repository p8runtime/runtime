#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's values and PHP 8.4's operations on them: scalars held inline,
//! strings and arrays shared until written, references where a variable is
//! one. Conversions and comparisons follow zend_operators.c.

pub mod sort;
pub mod args;
pub mod array;
pub mod ext;
pub mod fast;
pub mod frozen;
pub mod gc;
pub mod info;
pub mod number;
pub mod numeric;
pub mod object;
pub mod ops;
pub mod os;
pub mod reflect;
pub mod symbol;
/// The builtins' declared signatures (for reflection).
pub use p8_stubs as stubs;
pub mod stream;
pub mod user_stream;
pub mod trace;
pub mod types;
pub mod string;
pub mod value;

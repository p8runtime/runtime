#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8's values and PHP 8.4's operations on them: scalars held inline,
//! strings and arrays shared until written, references where a variable is
//! one. Conversions and comparisons follow zend_operators.c.

pub mod array;
pub mod ext;
pub mod number;
pub mod numeric;
pub mod ops;
pub mod string;
pub mod value;

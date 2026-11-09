//! The contract between the generator (`gen.rs`, which emits code against
//! it) and the harness (`main.rs` + `ffi.rs`, which implements the helpers
//! and calls the generated functions). Both binaries compile this file.
//!
//! A value crosses the boundary as p8's `Value` is laid out: two words, the
//! tag (`repr(u64)`, the numbers below) and the payload. In registers it is
//! two `i64`s, in memory 16 bytes, tag first. A function's result is the
//! same pair; the tag `T_ERR` (no value has it: Rust's own niche for
//! `R<Value>`) means a fault, whose message the harness keeps.
//!
//! Every generated function and every helper is the platform's C calling
//! convention (System V on x86-64): `(cx, st, ...)` as pointers, values as
//! two words or as a pointer to 16 bytes, results in rax:rdx.

// Each binary uses its part of the contract.
#![allow(dead_code)]

pub const T_NULL: i64 = 0;
pub const T_INT: i64 = 2;
pub const T_UNDEF: i64 = 4;
/// Tags below this hold nothing to release (`Value::is_plain`).
pub const FIRST_HEAP: i64 = 6;
pub const T_REF: i64 = 8;
pub const T_OBJECT: i64 = 9;
/// A fault (not a value's tag).
pub const T_ERR: i64 = 10;

/// The compiled variables the helpers' warnings name, by index (V4b's
/// `Var` statics).
pub mod var {
    pub const N7: u32 = 0;
    pub const I7: u32 = 1;
    pub const I8: u32 = 2;
    pub const I10: u32 = 3;
    pub const X10: u32 = 4;
    pub const FA: u32 = 5;
}

/// Property names a fill helper looks up, by index.
pub const PROP_P: u32 = 0;

/// The programs compiled, and their generated `t`.
pub const PROGRAMS: [(&str, &str); 3] = [("add", "clif_t_add"), ("call", "clif_t_call"), ("prop_read", "clif_t_prop_read")];

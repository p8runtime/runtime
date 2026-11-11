#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! The contract between the code p8's native backend generates
//! (`p8-native-gen`, which writes it) and the boundary crate that runs it
//! (`p8-native`, the only crate allowed `unsafe`, #402): every number both
//! sides must agree on, written once, here. The boundary checks the ones
//! that describe Rust's own layouts against the live layouts before any
//! native code runs.
//!
//! # A native function
//!
//! `extern "C" fn(ctx: *mut Ctx) -> u64`, one per PHP function compiled.
//! `Ctx` is the boundary's `repr(C)` struct, read at the `CTX_*` offsets.
//! The function runs its PHP function's frame from pc 0 on the frame's
//! registers (16-byte values, [`VALUE_SIZE`], at `ctx.regs`), touching
//! plain values only (tags below [`tag::FIRST_HEAP`]: no count to keep,
//! nothing to release), and returns `reason << 32 | pc` ([`exit`]): where
//! the interpreter takes the frame over. It calls one helper, [`CALL`].
//!
//! # The executable
//!
//! The generated object also holds the table of native functions, the
//! package and a C `main`, which passes [`ENTRY`] the image ([`image`]).

pub use p8_runtime::value::tag;

/// A value's size, and where its tag and payload words are.
pub const VALUE_SIZE: i32 = 16;
pub const TAG: i32 = 0;
pub const PAYLOAD: i32 = 8;

/// `Ctx`'s fields, by offset: the frame's window (a pointer to its first
/// register; read again after every helper call, which may move the
/// register file); the isolate's interrupt word (a `u64`, 0 while none is
/// due), tested at every backward jump.
pub const CTX_REGS: i32 = 0;
pub const CTX_INTERRUPT: i32 = 8;

/// What a native function returns: `reason << 32 | pc`.
pub mod exit {
    /// At a return, which the interpreter runs.
    pub const RETURN: u64 = 0;
    /// At an instruction whose operands the code has nothing for (a
    /// value of another type, an overflow, a reference, an interrupt
    /// due): the interpreter runs it, the registers as before it.
    pub const GUARD: u64 = 1;
    /// After the call at pc, which left an error or a destructor due.
    pub const PENDING: u64 = 2;
    /// At the call at pc, which threw.
    pub const THREW: u64 = 3;
}

/// The helper for a call (`Op::Call`): `extern "C" fn(ctx: *mut Ctx, pc:
/// u32) -> u32`, returning one of [`call`].
pub const CALL: &str = "p8n_call";

/// What [`CALL`] returns.
pub mod call {
    /// Done: the result is in the call's register; go on after it.
    pub const DONE: u32 = 0;
    /// Not run: hand the frame back at the call (`exit::GUARD`).
    pub const NOT_TAKEN: u32 = 1;
    /// Done, something due: `exit::PENDING`.
    pub const PENDING: u32 = 2;
    /// Threw: `exit::THREW`.
    pub const THREW: u32 = 3;
}

/// The boundary's entry: `extern "C" fn(argc: c_int, argv: *const *const
/// c_char, image: *const Image) -> c_int`, called by the generated `main`.
pub const ENTRY: &str = "p8_native_main";
/// The generated `main`'s name (the C runtime's entry calls it).
pub const MAIN: &str = "main";

/// The image the generated `main` passes: eight-byte fields, by offset.
pub mod image {
    pub const MAGIC: i32 = 0;
    pub const VERSION: i32 = 8;
    /// How many native functions there are.
    pub const COUNT: i32 = 16;
    /// The functions' addresses, `COUNT` of them, by id.
    pub const FUNCTIONS: i32 = 24;
    /// For each id, which function of the package it is: two `u32`s, the
    /// file (its record's number in the package's index) and the function
    /// (its index in the file's `Program::functions`).
    pub const KEYS: i32 = 32;
    /// The package's bytes and their length.
    pub const PACKAGE: i32 = 40;
    pub const PACKAGE_LEN: i32 = 48;
    pub const SIZE: usize = 56;
    /// `p8native`, and the version of this contract: an image of another
    /// contract is refused.
    pub const MAGIC_WORD: u64 = u64::from_le_bytes(*b"p8native");
    pub const VERSION_WORD: u64 = 1;
}

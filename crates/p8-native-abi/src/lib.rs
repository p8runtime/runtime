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
//! the interpreter takes the frame over. For anything else it calls one
//! helper, [`HELP`], which has the interpreter run that instruction, and
//! goes on at the pc the helper gives.
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
    /// At an instruction the code leaves to the interpreter, the registers
    /// as before it (a pc the helper gave past the code: never, in code
    /// the generator made).
    pub const GUARD: u64 = 1;
    /// The interpreter goes on from what the helper's run gave (it left
    /// the frame, or pushed a callee it did not run nested; the pc is 0).
    pub const LEFT: u64 = 2;
}

/// The one helper: `extern "C" fn(ctx: *mut Ctx, op: u32, pc: u32) ->
/// u64`, the instruction at `pc` run by the interpreter ([`op`]), returning
/// `status << 32 | next` ([`help`]).
pub const HELP: &str = "p8n_help";

/// What [`HELP`] runs.
pub mod op {
    /// An `Op::Call`: a resolved user function directly, else as `STEP`.
    pub const CALL: u32 = 0;
    /// Any instruction, as the interpreter's loop runs it out of line.
    pub const STEP: u32 = 1;
}

/// What [`HELP`] returns, in its high word.
pub mod help {
    /// The instruction ran: go on at `next` (the low word).
    pub const NEXT: u64 = 0;
    /// It ran, and the frame is the interpreter's: `exit::LEFT`.
    pub const LEFT: u64 = 2;
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
    /// The build of p8 that made the image: its engine key (16 bytes, the
    /// one its packages carry, `Platform::engine_key`). The runtime library
    /// refuses an image of another build.
    pub const ENGINE: i32 = 56;
    pub const SIZE: usize = 72;
    /// `p8native`, and the version of this contract: an image of another
    /// contract is refused.
    pub const MAGIC_WORD: u64 = u64::from_le_bytes(*b"p8native");
    pub const VERSION_WORD: u64 = 2;
}

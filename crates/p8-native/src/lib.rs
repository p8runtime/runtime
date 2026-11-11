//! THE BOUNDARY (#402): the only p8 crate without `#![forbid(unsafe_code)]`,
//! and the only place Rust calls machine code p8 generated or exposes
//! functions to it. Review it as a security boundary; this comment is the
//! audit guide.
//!
//! # What this crate is
//!
//! The runtime library of a p8-built executable (`libp8_native.a`):
//! `php2 --compile <dir> --native <exe>` links it with an object file
//! Cranelift wrote (`p8-native-gen`), which holds the native functions,
//! the image (`p8_native_abi::image`: their table, which package function
//! each is, the package's bytes) and a C `main` that calls [`p8_native_main`]
//! with the image. Everything else runs in safe Rust: the CLI's run of the
//! script (`php_cli::native::run`), and the interpreter, which enters a
//! native function through [`Code`] (`p8::native::NativeCode`) and runs
//! whatever the native code hands back (`p8-interp`'s `native.rs`).
//!
//! # The `unsafe`, all of it, and who guarantees what it assumes
//!
//! 1. [`p8_native_main`] reads the image through the pointer `main`
//!    passes: the generated object's own read-only data, built by
//!    `p8-native-gen` to the layout `p8_native_abi::image` names and
//!    [`Image`] declares (checked by `const` asserts below). The magic and
//!    version words are read and checked before any other field.
//!    Guaranteed by: the code generator and the linker (the object and
//!    this library are linked into one executable by `php2`).
//! 2. It reads the C `argv` the C runtime passed `main`: `argc` pointers
//!    to NUL-terminated strings. Guaranteed by: the C runtime.
//! 3. [`Code::run`] calls a native function: `extern "C" fn(*mut Ctx) ->
//!    u64`, the address from the image's table, by an id the interpreter
//!    got from the same image's keys (an id at or past the table's end is
//!    refused before the call). It assumes what the code generator
//!    guarantees: the function reads `Ctx` at the `CTX_*` offsets only,
//!    and touches only the frame's window (registers 0..256 from
//!    `ctx.regs`, which `NativeFrame::window` gave it whole), and only
//!    plain values (tags below `tag::FIRST_HEAP`: it never copies, drops
//!    or writes over a value holding memory: such a value fails its
//!    guards and the frame is handed back), and calls nothing but
//!    [`p8n_call`]. The words it writes are a valid `Value` of the tag it
//!    writes: checked for every tag by [`check_layout`] at startup.
//! 4. [`p8n_call`] turns the `*mut Ctx` back into `&mut Ctx`: the pointer
//!    `run` passed, which the native code passes on unchanged, while `run`
//!    is still on the stack below it (so it is live and nothing else
//!    holds it), and `Ctx::frame` back into the `NativeFrame` `run` was
//!    given. After the call it stores the window's address again (the
//!    call may have moved the register file), which the native code
//!    reloads after every call: so no pointer it holds outlives a change
//!    by Rust to what it points at.
//! 5. [`check_layout`] reads and builds values as their two words, for
//!    the plain tags only, to confirm the layout `p8_native_abi` and the
//!    code generator assume is the one this build of Rust gave `Value`.
//!
//! Every `unsafe` block says which of these it is (`// SAFETY:`); clippy's
//! `undocumented_unsafe_blocks` is denied, and `p8`'s `safety` test checks
//! that this is the only crate allowed `unsafe` and that every block here
//! carries its comment.
//!
//! # Not here
//!
//! No executable memory is made or mapped: machine code exists only in
//! executables linked ahead of time (#387). No helper takes a value from
//! native code or gives one to it: values cross only as the frame's
//! registers, plain ones.

#![deny(unsafe_op_in_unsafe_fn)]
#![deny(clippy::undocumented_unsafe_blocks)]

use p8::native::{CallDone, Exit, NativeCode, NativeFrame};
use p8_native_abi::{self as abi, call, exit, image, tag};
use p8_runtime::value::Value;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::{self, offset_of};
use std::sync::atomic::{AtomicU64, Ordering};

// The executable's allocator: php2's (see php-cli).
#[global_allocator]
static ALLOC: rusty_alloc_api::RustyAlloc = rusty_alloc_api::RustyAlloc;

/// What native code is passed: the frame's window, the interrupt word,
/// and (opaque to it) the frame, for [`p8n_call`].
#[repr(C)]
pub struct Ctx {
    regs: *mut Value,
    interrupt: *const AtomicU64,
    frame: *mut c_void,
}

const _: () = {
    assert!(offset_of!(Ctx, regs) == abi::CTX_REGS as usize);
    assert!(offset_of!(Ctx, interrupt) == abi::CTX_INTERRUPT as usize);
    assert!(mem::size_of::<Value>() == abi::VALUE_SIZE as usize);
    assert!(mem::size_of::<AtomicU64>() == 8);
};

/// A native function.
type NativeFn = unsafe extern "C" fn(*mut Ctx) -> u64;

/// The image the generated `main` passes (`p8_native_abi::image`).
#[repr(C)]
pub struct Image {
    magic: u64,
    version: u64,
    count: u64,
    functions: *const NativeFn,
    keys: *const [u32; 2],
    package: *const u8,
    package_len: u64,
}

const _: () = {
    assert!(offset_of!(Image, magic) == image::MAGIC as usize);
    assert!(offset_of!(Image, version) == image::VERSION as usize);
    assert!(offset_of!(Image, count) == image::COUNT as usize);
    assert!(offset_of!(Image, functions) == image::FUNCTIONS as usize);
    assert!(offset_of!(Image, keys) == image::KEYS as usize);
    assert!(offset_of!(Image, package) == image::PACKAGE as usize);
    assert!(offset_of!(Image, package_len) == image::PACKAGE_LEN as usize);
    assert!(mem::size_of::<Image>() == image::SIZE);
    assert!(mem::size_of::<NativeFn>() == 8);
};

/// The executable's native functions, and how they ran.
pub struct Code {
    functions: &'static [NativeFn],
    /// Native functions entered.
    entries: AtomicU64,
    /// Frames handed back at a return.
    returns: AtomicU64,
    /// Frames handed back at an instruction the code did not take (a
    /// guard: the interpreter ran it, and the rest of the frame).
    guards: AtomicU64,
}

impl NativeCode for Code {
    fn run(&self, id: u32, frame: &mut NativeFrame<'_, '_>) -> Exit {
        let Some(&f) = self.functions.get(id as usize) else {
            // Not one of ours: the interpreter runs the function.
            return Exit::At(0);
        };
        self.entries.fetch_add(1, Ordering::Relaxed);
        let interrupt: *const AtomicU64 = frame.interrupt();
        let mut ctx = Ctx { regs: std::ptr::null_mut(), interrupt, frame: std::ptr::from_mut(frame).cast() };
        // (Taken after `frame` went into `ctx`: nothing touches the frame
        // again until the native code calls back or returns.)
        // SAFETY: (4): `ctx.frame` is `frame`, borrowed for this call.
        ctx.regs = unsafe { frame_of(&mut ctx) }.window().as_mut_ptr();
        // SAFETY: (3): `f` is the image's function `id` (checked in range
        // above), of the declared signature; `ctx` lives across the call.
        let r = unsafe { f(&mut ctx) };
        let pc = r as u32;
        match r >> 32 {
            exit::RETURN => {
                self.returns.fetch_add(1, Ordering::Relaxed);
                Exit::At(pc)
            }
            exit::PENDING => Exit::Pending(pc),
            exit::THREW => Exit::Threw(pc),
            _ => {
                self.guards.fetch_add(1, Ordering::Relaxed);
                Exit::At(pc)
            }
        }
    }
}

/// The frame `ctx` was made for.
///
/// # Safety
/// `ctx.frame` is the `NativeFrame` [`Code::run`] put there, and `run`
/// is still running (4).
unsafe fn frame_of<'c>(ctx: &'c mut Ctx) -> &'c mut NativeFrame<'c, 'c> {
    // SAFETY: (4): the caller's promise; the lifetimes are no longer than
    // the borrow of `ctx`, which `run` outlives.
    unsafe { &mut *ctx.frame.cast::<NativeFrame<'c, 'c>>() }
}

/// The call helper (`p8_native_abi::CALL`): the `Op::Call` at `pc` of
/// the frame `ctx` runs on, run whole by the interpreter.
///
/// # Safety
/// `ctx` is the pointer [`Code::run`] passed the native function (4).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn p8n_call(ctx: *mut Ctx, pc: u32) -> u32 {
    // SAFETY: (4): the native code passes on the `ctx` `run` gave it,
    // while `run` is on the stack below.
    let ctx = unsafe { &mut *ctx };
    // SAFETY: (4): `ctx.frame` is `run`'s frame.
    let frame = unsafe { frame_of(ctx) };
    let done = frame.call(pc);
    let regs = frame.window().as_mut_ptr();
    ctx.regs = regs;
    match done {
        CallDone::Done => call::DONE,
        CallDone::NotTaken => call::NOT_TAKEN,
        CallDone::Pending => call::PENDING,
        CallDone::Threw => call::THREW,
    }
}

/// A value's two words, as native code reads them.
fn words(v: &Value) -> [u64; 2] {
    // SAFETY: (5): a `Value` is 16 bytes (asserted above) of two words, a
    // payload word in every variant (`value.rs`'s invariant), so every
    // byte is initialised; read, not moved: the value is untouched.
    unsafe { std::ptr::read(std::ptr::from_ref(v).cast::<[u64; 2]>()) }
}

/// The layout native code assumes, against the one this build has:
/// every plain tag read from a live value as the words `plain_words`
/// says, and a value native code writes (its tag and payload) read back
/// as the same value. Panics (before any native code runs) otherwise.
pub fn check_layout() {
    let samples = [Value::NULL, Value::TRUE, Value::FALSE, Value::int(-7), Value::int(i64::MIN), Value::float(-1.5), Value::float(f64::NAN), Value::UNDEF];
    for v in &samples {
        let want = v.plain_words().expect("a plain value");
        assert_eq!(words(v), want, "the layout of {v:?}");
        assert!(want[0] < tag::FIRST_HEAP);
    }
    for (t, p) in [(tag::NULL, 0), (tag::BOOL, 1), (tag::BOOL, 0), (tag::INT, 42u64), (tag::INT, (-3i64) as u64), (tag::FLOAT, 2.5f64.to_bits()), (tag::UNDEF, 0)] {
        // SAFETY: (5): a plain tag with its payload word is what that
        // variant is (`value.rs`), and a plain value has nothing to drop.
        let v: Value = unsafe { mem::transmute::<[u64; 2], Value>([t, p]) };
        assert_eq!(v.plain_words(), Some([t, p]), "a value native code writes");
    }
    assert_eq!(Value::int(5).as_int(), Some(5));
}

/// The entry the generated `main` calls with the image: check the
/// layout and the image, then run the script as php2 runs one, with the
/// image's package and its native functions. `--native-counts` first
/// says on stderr, at the end, how the native code ran.
///
/// # Safety
/// `argv` is `main`'s (2), `image` the generated object's (1).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn p8_native_main(argc: c_int, argv: *const *const c_char, image: *const Image) -> c_int {
    check_layout();
    // SAFETY: (1): the generated object's image; magic and version are the
    // first two words of any version of it.
    let (magic, version) = unsafe { ((*image).magic, (*image).version) };
    if magic != image::MAGIC_WORD || version != image::VERSION_WORD {
        eprintln!("this executable's native code was made for another p8 runtime");
        return 70;
    }
    // SAFETY: (1): this version's image, read-only data of the object.
    let image = unsafe { &*image };
    let count = usize::try_from(image.count).expect("a count");
    let package_len = usize::try_from(image.package_len).expect("a length");
    // SAFETY: (1): `count` function addresses and `count` keys, and the
    // package's bytes, in the object's data, for the life of the process.
    let (functions, keys, package) = unsafe {
        (
            std::slice::from_raw_parts(image.functions, count),
            std::slice::from_raw_parts(image.keys, count),
            std::slice::from_raw_parts(image.package, package_len),
        )
    };
    let mut args: Vec<String> = (1..usize::try_from(argc).unwrap_or(0))
        .map(|i| {
            // SAFETY: (2): `argv[i]` for i < argc is a NUL-terminated string.
            let s = unsafe { CStr::from_ptr(*argv.add(i)) };
            s.to_string_lossy().into_owned()
        })
        .collect();
    let counts = args.first().is_some_and(|a| a == "--native-counts");
    if counts {
        args.remove(0);
    }
    let code: &'static Code = Box::leak(Box::new(Code { functions, entries: AtomicU64::new(0), returns: AtomicU64::new(0), guards: AtomicU64::new(0) }));
    let native = php_cli::native::Embedded { code, keys: keys.iter().map(|k| (k[0], k[1])).collect(), package: package.to_vec() };
    let status = php_cli::native::run(args, native);
    if counts {
        eprintln!(
            "p8 native: {} functions, {} entries, {} returns, {} handed back at a guard",
            functions.len(),
            code.entries.load(Ordering::Relaxed),
            code.returns.load(Ordering::Relaxed),
            code.guards.load(Ordering::Relaxed)
        );
    }
    status
}

#[cfg(test)]
mod tests {
    /// The layout native code assumes is this build's.
    #[test]
    fn the_layout_is_the_one_native_code_assumes() {
        super::check_layout();
    }
}

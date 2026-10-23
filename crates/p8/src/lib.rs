#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8: an embeddable PHP 8.4 engine (APS 3 in p8runtime/aps: engine and
//! hosts). Hosts use this crate only; the
//! crates behind it (syntax, compiler, bytecode, runtime, interpreter,
//! extensions) are not part of the API.
//!
//! ```no_run
//! struct Stdout;
//! impl p8::Host for Stdout {
//!     fn write(&mut self, bytes: &[u8]) {
//!         use std::io::Write;
//!         let _ = std::io::stdout().write_all(bytes);
//!     }
//! }
//!
//! let platform = p8::Platform::new(&[p8::extensions::STANDARD]); // once per process
//! let mut isolate = platform.new_isolate();                      // once per thread
//! let script = platform.compile(b"<?php echo 1 + 2;", "a.php");
//! let mut host = Stdout;
//! let mut context = isolate.new_context(&mut host);              // once per request
//! let status = match &script {
//!     Ok(script) => context.run(script),
//!     Err(error) => context.fail(error),
//! };
//! ```

use p8_runtime::ext::Registry;
use std::sync::Arc;

pub use p8_interp::{Host, InterruptHandle};
pub use p8_runtime::ext::Extension;

/// The extensions p8 ships.
pub mod extensions {
    pub use p8_ext_standard::EXTENSION as STANDARD;
}

/// Process-wide state shared by every isolate: the registered extensions.
pub struct Platform {
    registry: Arc<Registry>,
}

impl Platform {
    pub fn new(extensions: &[Extension]) -> Platform {
        Platform {
            registry: Arc::new(Registry::new(extensions)),
        }
    }

    /// An engine for the calling thread.
    pub fn new_isolate(&self) -> Isolate {
        Isolate {
            inner: p8_interp::Isolate::new(Arc::clone(&self.registry)),
        }
    }

    /// Compile a file's source.
    pub fn compile(&self, source: &[u8], file: &str) -> Result<Script, Error> {
        let arena = bumpalo::Bump::new();
        let lexer = p8_syntax::lexer::Lexer::new(source);
        let mut parser = p8_syntax::parser::Parser::new(lexer, &arena);
        let program = parser.parse_program();
        if let Some(error) = program.errors.first() {
            let line = error
                .span
                .line_info(source)
                .map_or(1, |info| info.line as u32);
            return Err(Error {
                kind: ErrorKind::Parse,
                message: error.message.to_string(),
                file: file.into(),
                line,
            });
        }
        match p8_compiler::Compiler::new(source, file, &self.registry).compile(program.statements) {
            Ok(program) => Ok(Script { program }),
            Err(e) => Err(Error {
                kind: ErrorKind::Fatal,
                message: e.message,
                file: file.into(),
                line: e.line,
            }),
        }
    }

    /// A script's bytecode, as text (for `--dump`).
    pub fn disassemble(&self, script: &Script) -> String {
        let program = &script.program;
        let mut out = String::new();
        for f in std::iter::once(&program.main).chain(&program.functions) {
            out.push_str(&format!(
                "function {} (regs {}, consts at {}: {:?})\n",
                String::from_utf8_lossy(f.name.as_bytes()),
                f.nregs,
                f.consts_at,
                f.consts
            ));
            for (pc, instr) in f.code.iter().enumerate() {
                out.push_str(&format!("  {pc:4} L{:<4} {instr:?}\n", f.lines[pc]));
            }
        }
        out
    }
}

/// A compiled file: immutable, and `Send + Sync`, so one compile serves
/// every isolate on every thread.
pub struct Script {
    program: p8_compiler::Program,
}

/// A file that does not compile.
#[derive(Debug, Clone)]
pub struct Error {
    pub kind: ErrorKind,
    pub message: String,
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// PHP's "Parse error".
    Parse,
    /// PHP's "Fatal error" raised while compiling.
    Fatal,
}

/// One engine on one thread; it runs one context (request) at a time.
pub struct Isolate {
    inner: p8_interp::Isolate,
}

impl Isolate {
    /// A handle that stops this isolate's running request from any thread
    /// (a host's watchdog holds one).
    pub fn interrupt_handle(&self) -> InterruptHandle {
        self.inner.interrupt_handle()
    }

    /// Start a request whose output goes to `host`. The context borrows the
    /// isolate until it is dropped, which ends the request.
    pub fn new_context<'i>(&'i mut self, host: &'i mut dyn Host) -> Context<'i> {
        Context {
            inner: self.inner.new_context(host),
        }
    }
}

/// One request.
pub struct Context<'i> {
    inner: p8_interp::Context<'i>,
}

impl Context<'_> {
    /// Run a script's top-level code; returns the exit status.
    pub fn run(&mut self, script: &Script) -> i32 {
        for f in &script.program.functions {
            if let Err(message) = self.inner.declare(Arc::clone(f)) {
                self.inner
                    .echo_and_flush(format!("\nFatal error: {message} in {}\n", f.file).as_bytes());
                return 255;
            }
        }
        self.inner.run_main(Arc::clone(&script.program.main))
    }

    /// Report a file that did not compile, as PHP does; returns the exit status.
    pub fn fail(&mut self, error: &Error) -> i32 {
        let label = match error.kind {
            ErrorKind::Parse => "Parse error",
            ErrorKind::Fatal => "Fatal error",
        };
        let text = format!(
            "\n{label}: {} in {} on line {}\n",
            error.message, error.file, error.line
        );
        self.inner.echo_and_flush(text.as_bytes());
        255
    }
}

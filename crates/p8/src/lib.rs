#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! p8: an embeddable PHP 8.4 engine (docs/design/engine-v2.md, "p8: the
//! engine, and the hosts that drive it"). Hosts use this crate only; the
//! crates behind it (syntax, compiler, bytecode, runtime, interpreter,
//! extensions) are not part of the API.

use p8_runtime::ext::Registry;
use std::rc::Rc;
use std::sync::Arc;

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
        Platform { registry: Arc::new(Registry::new(extensions)) }
    }

    /// Compile PHP source and print its bytecode.
    pub fn dump(&self, source: &[u8], file: &str) -> Result<String, String> {
        let arena = bumpalo::Bump::new();
        let lexer = p8_syntax::lexer::Lexer::new(source);
        let mut parser = p8_syntax::parser::Parser::new(lexer, &arena);
        let program = parser.parse_program();
        if let Some(error) = program.errors.first() {
            return Err(error.message.to_string());
        }
        let compiled = p8_compiler::Compiler::new(source, file, &self.registry)
            .compile(program.statements)
            .map_err(|e| e.message)?;
        let mut out = String::new();
        for f in std::iter::once(&compiled.main).chain(&compiled.functions) {
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
        Ok(out)
    }

    /// Compile and run PHP source as one request; returns the exit status.
    pub fn run(&self, source: &[u8], file: &str) -> i32 {
        let arena = bumpalo::Bump::new();
        let lexer = p8_syntax::lexer::Lexer::new(source);
        let mut parser = p8_syntax::parser::Parser::new(lexer, &arena);
        let program = parser.parse_program();
        if let Some(error) = program.errors.first() {
            let line = error.span.line_info(source).map_or(1, |info| info.line);
            print!("\nParse error: {} in {} on line {}\n", error.message, file, line);
            return 255;
        }
        let compiled = match p8_compiler::Compiler::new(source, file, &self.registry).compile(program.statements) {
            Ok(p) => p,
            Err(e) => {
                print!("\nFatal error: {} in {} on line {}\n", e.message, file, e.line);
                return 255;
            }
        };
        let mut vm = p8_interp::Vm::new(Arc::clone(&self.registry));
        for f in compiled.functions {
            if let Err(message) = vm.declare(f) {
                print!("\nFatal error: {} in {}\n", message, file);
                return 255;
            }
        }
        vm.run_main(Rc::clone(&compiled.main))
    }
}

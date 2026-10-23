//! Engine v2: the new core (docs/design/engine-v2.md). It is built next to
//! the current engine and replaces it at php-src parity.

pub mod array;
pub mod builtins;
pub mod code;
pub mod compiler;
pub mod ops;
pub mod string;
pub mod value;
pub mod vm;

use std::rc::Rc;

/// Compile PHP source and print its bytecode (php2 --dump).
pub fn dump_source(source: &[u8], file: &str) -> Result<String, String> {
    let arena = bumpalo::Bump::new();
    let lexer = crate::parser::lexer::Lexer::new(source);
    let mut parser = crate::parser::parser::Parser::new(lexer, &arena);
    let program = parser.parse_program();
    if let Some(error) = program.errors.first() {
        return Err(error.message.to_string());
    }
    let compiled = compiler::Compiler::new(source, file).compile(program.statements).map_err(|e| e.message)?;
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
pub fn run_source(source: &[u8], file: &str) -> i32 {
    let arena = bumpalo::Bump::new();
    let lexer = crate::parser::lexer::Lexer::new(source);
    let mut parser = crate::parser::parser::Parser::new(lexer, &arena);
    let program = parser.parse_program();
    if let Some(error) = program.errors.first() {
        let line = error.span.line_info(source).map_or(1, |info| info.line);
        print!("\nParse error: {} in {} on line {}\n", error.message, file, line);
        return 255;
    }
    let compiled = match compiler::Compiler::new(source, file).compile(program.statements) {
        Ok(p) => p,
        Err(e) => {
            print!("\nFatal error: {} in {} on line {}\n", e.message, file, e.line);
            return 255;
        }
    };
    let mut vm = vm::Vm::new();
    for f in compiled.functions {
        if let Err(message) = vm.declare(f) {
            print!("\nFatal error: {} in {}\n", message, file);
            return 255;
        }
    }
    vm.run_main(Rc::clone(&compiled.main))
}

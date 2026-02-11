use bumpalo::Bump;
use clap::Parser;
use php_rs::compiler::emitter::Emitter;
use php_rs::compiler::chunk::UserFunc;
use php_rs::core::interner::Interner;
use php_rs::core::value::Val;
use php_rs::parser::lexer::Lexer;
use php_rs::parser::parser::{detect_parser_mode, Parser as PhpParser};
use std::fs;
use std::path::PathBuf;

#[derive(Parser)]
struct Cli {
    file: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let source = fs::read_to_string(&cli.file)?;
    let source_bytes = source.as_bytes();

    let arena = Bump::new();
    let lexer = Lexer::new(source_bytes);
    let mode = detect_parser_mode(source_bytes, Some(cli.file.as_path()));
    let mut parser = PhpParser::new_with_mode(lexer, &arena, mode);

    let program = parser.parse_program();

    if !program.errors.is_empty() {
        for error in program.errors {
            println!("{}", error.to_human_readable(source_bytes));
        }
        return Ok(());
    }
    if mode == php_rs::parser::parser::ParserMode::Phpx {
        if let Err(errors) = php_rs::phpx::typeck::check_program_with_path(
            &program,
            source_bytes,
            Some(cli.file.as_path()),
        ) {
            let rendered = php_rs::phpx::typeck::format_type_errors(&errors, source_bytes);
            println!("{}", rendered);
            return Ok(());
        }
    }

    let mut interner = Interner::new();
    let emitter = Emitter::new(source_bytes, &mut interner);
    let (chunk, _has_error) = emitter.compile(program.statements);

    println!("=== Bytecode ===");
    for (i, op) in chunk.code.iter().enumerate() {
        println!("{:4}: {:?}", i, op);
    }

    println!("\n=== Constants ===");
    for (i, val) in chunk.constants.iter().enumerate() {
        println!("{}: {:?}", i, val);
        if let Val::Resource(any) = val {
            if let Some(func) = any.as_ref().downcast_ref::<UserFunc>() {
                println!("  -> user func params: {}", func.params.len());
                for (p_i, p) in func.params.iter().enumerate() {
                    println!(
                        "     param[{}]: name={:?} by_ref={} variadic={}",
                        p_i, p.name, p.by_ref, p.is_variadic
                    );
                }
                for (op_i, op) in func.chunk.code.iter().enumerate() {
                    println!("     {:4}: {:?}", op_i, op);
                }
                if !func.chunk.constants.is_empty() {
                    println!("     constants:");
                    for (k, v) in func.chunk.constants.iter().enumerate() {
                        println!("       [{}] {:?}", k, v);
                    }
                }
            }
        }
    }

    println!("\n=== Catch Table ===");
    for (i, entry) in chunk.catch_table.iter().enumerate() {
        println!(
            "{}: start={} end={} target={} catch_type={:?} finally_target={:?} finally_end={:?}",
            i,
            entry.start,
            entry.end,
            entry.target,
            entry.catch_type,
            entry.finally_target,
            entry.finally_end
        );
    }

    println!("\n=== Symbols ===");
    let mut i = 0u32;
    loop {
        let sym = php_rs::core::value::Symbol(i);
        let Some(bytes) = interner.lookup(sym) else {
            break;
        };
        println!("{}: {}", i, String::from_utf8_lossy(bytes));
        i += 1;
    }

    Ok(())
}

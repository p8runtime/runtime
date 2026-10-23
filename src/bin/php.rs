#![forbid(unsafe_code)] // memory safety is the point of this runtime; see SECURITY.md
mod common;

use clap::Parser;
use common::{create_engine, execute_source, run_script};
use php_rs::vm::engine::VM;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "php")]
#[command(about = "PHP Interpreter in Rust", long_about = None)]
struct Cli {
    /// Run interactively
    #[arg(short = 'a', long)]
    interactive: bool,

    /// Script file to run
    #[arg(name = "FILE")]
    file: Option<PathBuf>,

    /// Arguments to pass to the script
    #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
    args: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if cli.interactive {
        run_repl()?;
    } else if let Some(file) = cli.file {
        // The interpreter recurses (callbacks, nested generators, the
        // parser on deep nesting): give it a large stack. The VM turns
        // running out of it into PHP's "Maximum call stack size" Error.
        let args = cli.args.clone();
        let status = std::thread::Builder::new()
            .stack_size(php_rs::vm::engine::INTERPRETER_STACK_SIZE)
            .spawn(move || {
                php_rs::vm::engine::set_stack_budget(
                    php_rs::vm::engine::INTERPRETER_STACK_SIZE - 16 * 1024 * 1024,
                );
                run_script(&file, &args)
            })?
            .join()
            .map_err(|_| anyhow::anyhow!("interpreter thread panicked"))??;
        std::process::exit(status);
    } else {
        // If no arguments, show help
        use clap::CommandFactory;
        Cli::command().print_help()?;
    }

    Ok(())
}

fn run_repl() -> anyhow::Result<()> {
    let mut rl = DefaultEditor::new()?;
    if let Err(_) = rl.load_history("history.txt") {
        // No history file is fine
    }

    println!("Interactive shell");
    println!("Type 'exit' or 'quit' to quit");

    let engine_context = create_engine()?;
    let mut vm = VM::new(engine_context);

    loop {
        let readline = rl.readline("php > ");
        match readline {
            Ok(line) => {
                let line = line.trim();
                if line == "exit" || line == "quit" {
                    break;
                }
                rl.add_history_entry(line)?;

                let source_code = if line.starts_with("<?php") {
                    line.to_string()
                } else {
                    format!("<?php {}", line)
                };

                execute_source(source_code.as_bytes(), None, &mut vm);
            }
            Err(ReadlineError::Interrupted) => {
                println!("CTRL-C");
                break;
            }
            Err(ReadlineError::Eof) => {
                println!("CTRL-D");
                break;
            }
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        }
    }
    rl.save_history("history.txt")?;
    Ok(())
}

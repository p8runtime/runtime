#![forbid(unsafe_code)] // memory safety is the point of this runtime; see SECURITY.md
mod common;

use common::{create_engine, execute_source, run_code, run_script};
use php_rs::vm::engine::VM;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use std::path::PathBuf;

const USAGE: &str = "Usage: php [options] [-f] <file> [--] [args...]
   php [options] -r <code> [--] [args...]
   php [options] -a

  -a               Run as interactive shell
  -c <path>        Look for php.ini in this directory (ignored: no php.ini is read)
  -n               No configuration (ini) files will be used
  -d foo[=bar]     Define INI entry foo with value 'bar'
  -f <file>        Parse and execute <file>.
  -h               This help
  -m               Show compiled in modules
  -r <code>        Run PHP <code> without using script tags <?..?>
  -v               Version number
";

/// What the command line asks for, read the way PHP's CLI reads it: options
/// come first, then the script (or `-r` code), then the script's arguments.
struct Cli {
    ini: Vec<(String, String)>,
    action: Action,
    args: Vec<String>,
}

enum Action {
    File(PathBuf),
    Code(String),
    Interactive,
    Modules,
    Version,
    Help,
}

fn parse_cli(argv: &[String]) -> Result<Cli, String> {
    let mut ini = Vec::new();
    let mut action = None;
    let mut i = 0;
    // An option's value is the rest of the word or else the next word.
    let value = |i: &mut usize, flag: &str| -> Result<String, String> {
        let word = &argv[*i];
        if word.len() > 2 {
            return Ok(word[2..].to_string());
        }
        *i += 1;
        argv.get(*i)
            .cloned()
            .ok_or_else(|| format!("Option {} requires an argument", flag))
    };
    while i < argv.len() && action.is_none() {
        let word = &argv[i];
        match word.get(..2) {
            Some("-d") => {
                let setting = value(&mut i, "-d")?;
                let (name, val) = setting.split_once('=').unwrap_or((&setting, "1"));
                ini.push((name.to_string(), val.to_string()));
            }
            Some("-c") => {
                value(&mut i, "-c")?;
            }
            Some("-r") => action = Some(Action::Code(value(&mut i, "-r")?)),
            Some("-f") => action = Some(Action::File(PathBuf::from(value(&mut i, "-f")?))),
            Some("-n") | Some("-q") | Some("-e") | Some("-H") => {}
            Some("-a") => action = Some(Action::Interactive),
            Some("-m") => action = Some(Action::Modules),
            Some("-v") => action = Some(Action::Version),
            Some("-h") | Some("-?") => action = Some(Action::Help),
            Some("--") if word == "--" => {
                i += 1;
                break;
            }
            _ if word.starts_with('-') && word.len() > 1 => {
                return Err(format!("Unknown option: {}", word));
            }
            _ => action = Some(Action::File(PathBuf::from(word))),
        }
        i += 1;
    }
    let mut args: Vec<String> = argv[i.min(argv.len())..].to_vec();
    if args.first().is_some_and(|a| a == "--") {
        args.remove(0);
    }
    let action = match action {
        Some(action) => action,
        // Like PHP, a bare `--` with no script still needs one
        None => match args.is_empty() {
            true => Action::Help,
            false => Action::File(PathBuf::from(args.remove(0))),
        },
    };
    Ok(Cli { ini, action, args })
}

fn main() -> anyhow::Result<()> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let cli = match parse_cli(&argv) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("{}\n{}", message, USAGE);
            std::process::exit(1);
        }
    };
    let Cli { ini, action, args } = cli;
    match action {
        Action::Interactive => run_repl(),
        Action::Help => {
            print!("{}", USAGE);
            Ok(())
        }
        Action::Version => {
            println!("PHP 8.4.27 (cli) (php-rs)\nCopyright (c) The PHP Group");
            Ok(())
        }
        Action::Modules => {
            let engine = create_engine()?;
            println!("[PHP Modules]");
            let mut names = engine.registry.get_extensions();
            names.sort_by_key(|name| name.to_ascii_lowercase());
            for name in names {
                println!("{}", name);
            }
            println!("\n[Zend Modules]\n");
            Ok(())
        }
        Action::File(_) | Action::Code(_) => {
            // The interpreter recurses (callbacks, nested generators, the
            // parser on deep nesting): give it a large stack. The VM turns
            // running out of it into PHP's "Maximum call stack size" Error.
            let status = std::thread::Builder::new()
                .stack_size(php_rs::vm::engine::INTERPRETER_STACK_SIZE)
                .spawn(move || {
                    php_rs::vm::engine::set_stack_budget(
                        php_rs::vm::engine::INTERPRETER_STACK_SIZE - 16 * 1024 * 1024,
                    );
                    match action {
                        Action::Code(code) => run_code(&code, &args, &ini),
                        Action::File(file) => run_script(&file, &args, &ini),
                        _ => Ok(0),
                    }
                })?
                .join()
                .map_err(|_| anyhow::anyhow!("interpreter thread panicked"))??;
            std::process::exit(status);
        }
    }
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

#![forbid(unsafe_code)]
//! vmgen: a measured prototype of generated specialised handlers for p8
//! (#366, #365 phase 0). Compiles a PHP file with p8's compiler, translates
//! the function `t` (and what it calls) into handler-carrying instructions,
//! and runs `t($n)`.
//!
//!   vmgen <file.php> <n>          run t(n); the program's output on stdout
//!   vmgen <file.php> <n> --dump   also list the translated instructions (stderr)
//!   vmgen <file.php> --dump       list only
//!
//! Exit 0 when t ran, 2 when the file (or a function it needs) is outside
//! the subset (the reason on stderr), 1 for a fault while running.

mod kinds;
mod link;
mod ops;
mod translate;
mod vm;

#[allow(unused_imports)]
mod handlers {
    include!(concat!(env!("OUT_DIR"), "/handlers.rs"));
}

use p8_runtime::value::Value;
use std::process::ExitCode;

fn dump(linked: &link::Linked<'_>) {
    for f in &linked.funcs {
        eprintln!("function {} (regs {}, {} variables, {} consts)", f.name, f.nregs, f.ncv, f.consts.len());
        if let Some(why) = &f.refused {
            eprintln!("    REFUSED: {why}");
            continue;
        }
        for (pc, i) in f.code.iter().enumerate() {
            let name = handlers::TABLE.get(usize::from(i.id)).map_or("?", |e| e.2);
            eprintln!("    {pc:3} {name:<40} a={} b={} c={} d={} imm={}", i.a, i.b, i.c, i.d, i.imm);
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let want_dump = args.iter().any(|a| a == "--dump");
    let rest: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let Some(file) = rest.first() else {
        eprintln!("usage: vmgen <file.php> [n] [--dump]");
        return ExitCode::from(2);
    };
    let n: Option<i64> = rest.get(1).and_then(|s| s.parse().ok());
    let source = match std::fs::read(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{file}: {e}");
            return ExitCode::from(2);
        }
    };
    let program = match link::compile(&source, file) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{file}: {e}");
            return ExitCode::from(2);
        }
    };
    let pool = link::Pool::new(&program);
    let linked = match link::link(&program, &pool) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("refused: {e}");
            return ExitCode::from(2);
        }
    };
    if want_dump {
        dump(&linked);
    }
    let Some(n) = n else {
        return ExitCode::SUCCESS;
    };
    let Some(entry) = linked.funcs.iter().position(|f| f.name == "t") else {
        eprintln!("refused: no function t in {file}");
        return ExitCode::from(2);
    };
    if let Some(why) = &linked.funcs[entry].refused {
        eprintln!("refused: t: {why}");
        return ExitCode::from(2);
    }
    let mut st = vm::State::new(&linked);
    let mut regs = Vec::new();
    let result = vm::run(&mut st, &mut regs, entry as u32, vec![Value::int(n)]);
    ops::flush(&mut st);
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) if e.starts_with("refused") || e.contains("not in the prototype") => {
            eprintln!("refused: {e}");
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("fault: {e}");
            ExitCode::from(1)
        }
    }
}

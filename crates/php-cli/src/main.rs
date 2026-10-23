#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! The php command-line host for p8 (p8runtime/aps APS 3), named
//! php2 while p8 is built next to the legacy engine: `php2 script.php`.

use std::io::Write;

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let dump = args.first().is_some_and(|a| a == "--dump");
    if dump {
        args.remove(0);
    }
    let Some(path) = args.into_iter().next() else {
        eprintln!("usage: php2 [--dump] <file>");
        std::process::exit(64);
    };
    let source = match std::fs::read(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Could not open input file: {path} ({e})");
            std::process::exit(1);
        }
    };
    let platform = p8::Platform::new(&[p8::extensions::STANDARD]);
    let file = std::fs::canonicalize(&path).map_or(path.clone(), |p| p.to_string_lossy().into_owned());
    if dump {
        match platform.compile(&source, &file) {
            Ok(script) => print!("{}", platform.disassemble(&script)),
            Err(error) => {
                eprintln!("{}", error.message);
                std::process::exit(255);
            }
        }
        return;
    }
    // Deep PHP recursion needs a deep native stack: run on a thread with one.
    let status = std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let mut isolate = platform.new_isolate();
            let mut stdout = Stdout(std::io::BufWriter::with_capacity(64 * 1024, std::io::stdout()));
            let mut context = isolate.new_context(&mut stdout);
            match platform.compile(&source, &file) {
                Ok(script) => context.run(&script),
                Err(error) => context.fail(&error),
            }
        })
        .expect("interpreter thread")
        .join()
        .unwrap_or(255);
    std::process::exit(status);
}

/// The terminal: a request's output goes to stdout.
struct Stdout(std::io::BufWriter<std::io::Stdout>);

impl p8::Host for Stdout {
    fn write(&mut self, bytes: &[u8]) {
        let _ = self.0.write_all(bytes);
    }

    fn flush(&mut self) {
        let _ = self.0.flush();
    }
}

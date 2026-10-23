#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! The php command-line host for p8 (p8runtime/aps APS 3), named
//! php2 while p8 is built next to the legacy engine: `php2 script.php`.

use std::io::Write;

fn main() {
    let Some(Cli { dump, path, .. }) = Cli::parse(std::env::args().skip(1)) else {
        eprintln!("usage: php2 [--dump] [-d name=value]... <file> [args...]");
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

/// The command line, as `php` takes it: options, the file, then the
/// script's own arguments.
struct Cli {
    dump: bool,
    path: String,
}

impl Cli {
    fn parse(args: impl Iterator<Item = String>) -> Option<Cli> {
        let mut args = args.peekable();
        let mut dump = false;
        while let Some(arg) = args.next_if(|a| a.starts_with('-') && a != "-") {
            match arg.as_str() {
                "--dump" => dump = true,
                // `-d name=value`: p8 has no ini settings yet, so these are
                // accepted (php-src's runner passes several) and not applied.
                "-d" => {
                    args.next()?;
                }
                other if other.starts_with("-d") => {}
                _ => return None,
            }
        }
        Some(Cli { dump, path: args.next()? })
    }
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

#[cfg(test)]
mod tests {
    use super::Cli;

    fn parse(args: &[&str]) -> Option<(bool, String)> {
        Cli::parse(args.iter().map(|a| a.to_string())).map(|c| (c.dump, c.path))
    }

    /// What php-src's runner passes: settings, the file, the test's arguments.
    #[test]
    fn takes_the_runners_command_line() {
        let ran = parse(&["-d", "display_errors=1", "-dprecision=14", "-d", "x", "t.php", "a", "-b"]);
        assert_eq!(ran, Some((false, "t.php".to_string())));
        assert_eq!(parse(&["--dump", "t.php"]), Some((true, "t.php".to_string())));
        // An unknown option or a missing file is a usage error.
        assert_eq!(parse(&["-x", "t.php"]), None);
        assert_eq!(parse(&["-d"]), None);
        assert_eq!(parse(&["-d", "a=1"]), None);
    }
}

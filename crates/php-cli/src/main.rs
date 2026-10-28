#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! The php command-line host for p8 (p8runtime/aps APS 3), named
//! php2 while p8 is built next to the legacy engine: `php2 script.php`.

use std::io::Write;

fn main() {
    let Some(Cli { dump, lint, path, ini, args }) = Cli::parse(std::env::args().skip(1)) else {
        eprintln!("usage: php2 [--dump] [-l] [-d name=value]... <file> [args...]");
        std::process::exit(64);
    };
    let source = match std::fs::read(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Could not open input file: {path} ({e})");
            std::process::exit(1);
        }
    };
    let platform = p8::Platform::new(&[p8::extensions::STANDARD, p8::extensions::XML, p8::extensions::TOKENIZER, p8::extensions::PCRE, p8::extensions::MYSQLI]);
    let file = std::fs::canonicalize(&path).map_or(path.clone(), |p| p.to_string_lossy().into_owned());
    // -l: compile only, as `php -l` checks a file's syntax.
    if lint {
        match platform.compile(&source, &path) {
            Ok(_) => println!("No syntax errors detected in {path}"),
            Err(error) => {
                let label = if error.kind == p8::ErrorKind::Parse { "Parse error" } else { "Fatal error" };
                println!("\n{label}: {} in {path} on line {}\nErrors parsing {path}", error.message, error.line);
                std::process::exit(255);
            }
        }
        return;
    }
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
            for (name, value) in &ini {
                context.set_ini(name, value);
            }
            context.set_args(std::iter::once(path.clone()).chain(args).collect());
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
    /// -l: check the file's syntax, run nothing.
    lint: bool,
    path: String,
    /// `-d name=value` settings, in order (unknown names are ignored).
    ini: Vec<(String, String)>,
    /// The script's own arguments (after the file).
    args: Vec<String>,
}

impl Cli {
    fn parse(args: impl Iterator<Item = String>) -> Option<Cli> {
        let mut args = args.peekable();
        let mut dump = false;
        let mut lint = false;
        let mut ini = Vec::new();
        let mut setting = |text: &str| {
            let (name, value) = text.split_once('=').unwrap_or((text, "1"));
            ini.push((name.trim().to_string(), value.trim().trim_matches('"').to_string()));
        };
        while let Some(arg) = args.next_if(|a| a.starts_with('-') && a != "-") {
            match arg.as_str() {
                "--dump" => dump = true,
                "-l" => lint = true,
                "-d" => setting(&args.next()?),
                other if other.starts_with("-d") => setting(&other[2..]),
                _ => return None,
            }
        }
        let path = args.next()?;
        Some(Cli { dump, lint, path, ini, args: args.collect() })
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
        let cli = Cli::parse(["-d", "error_reporting=0", "-dprecision=14", "-d", "x", "t.php"].iter().map(|a| a.to_string())).expect("parsed");
        let ini: Vec<(&str, &str)> = cli.ini.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
        assert_eq!(ini, [("error_reporting", "0"), ("precision", "14"), ("x", "1")]);
        assert_eq!(parse(&["--dump", "t.php"]), Some((true, "t.php".to_string())));
        let lint = Cli::parse(["-l", "t.php"].iter().map(|a| a.to_string())).expect("parsed");
        assert!(lint.lint && !lint.dump && lint.path == "t.php");
        // An unknown option or a missing file is a usage error.
        assert_eq!(parse(&["-x", "t.php"]), None);
        assert_eq!(parse(&["-d"]), None);
        assert_eq!(parse(&["-d", "a=1"]), None);
    }
}

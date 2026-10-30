#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! The php command-line host for p8 (p8runtime/aps APS 3), named
//! php2 while p8 is built next to the legacy engine: `php2 script.php`.

use std::io::Write;

mod http;
mod mime_types;
mod server;
mod status_phrases;

fn main() {
    let Some(Cli { dump, lint, path, code, ini, args, serve, docroot, quiet }) = Cli::parse(std::env::args().skip(1)) else {
        eprintln!(
            "usage: php2 [--dump] [-l] [-d name=value]... [-f] <file> [--] [args...]\n       php2 [-d name=value]... -r <code> [--] [args...]\n       php2 [-d name=value]... [--] [args...]   (the script on stdin)\n       php2 -S <addr>:<port> [-t docroot] [-q] [router.php]"
        );
        std::process::exit(64);
    };
    // php -S: the built-in web server (the file, if any, is its router).
    if let Some(address) = serve {
        let config = server::Config { address, docroot, router: path, quiet, ini };
        let status = std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(move || server::run(config)).expect("server thread").join().unwrap_or(255);
        std::process::exit(status);
    }
    // The script: a file, -r's code, or what stdin gives (as php names them).
    let (source, path) = match (code, path) {
        (Some(code), _) => ([b"<?php ".as_slice(), code.as_bytes()].concat(), "Command line code".to_string()),
        (None, None) => {
            let mut source = Vec::new();
            let _ = std::io::Read::read_to_end(&mut std::io::stdin(), &mut source);
            (source, STDIN_NAME.to_string())
        }
        (None, Some(path)) => match std::fs::read(&path) {
            Ok(s) => (s, path),
            Err(e) => {
                eprintln!("Could not open input file: {path} ({e})");
                std::process::exit(1);
            }
        },
    };
    let named = path == STDIN_NAME || path == "Command line code";
    let platform = p8::Platform::new(&[p8::extensions::STANDARD, p8::extensions::XML, p8::extensions::TOKENIZER, p8::extensions::MBSTRING, p8::extensions::PCRE, p8::extensions::MYSQLI, p8::extensions::CURL, p8::extensions::GD, p8::extensions::ZIP, p8::extensions::FILEINFO, p8::extensions::SESSION, p8::extensions::PDO, p8::extensions::PDO_SQLITE, p8::extensions::OPENSSL])
        // A file included again (a template part per post) is not compiled again.
        .keep_compiled();
    let file = if named { path.clone() } else { std::fs::canonicalize(&path).map_or(path.clone(), |p| p.to_string_lossy().into_owned()) };
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
            // max_execution_time (0, the CLI's default: none): a watchdog
            // ends the run with PHP's fatal error once it has run that long.
            // (set_time_limit() calls are not followed yet.)
            let limit = ini.iter().rev().find(|(n, _)| n == "max_execution_time").and_then(|(_, v)| v.trim().parse::<u64>().ok()).unwrap_or(0);
            if limit > 0 {
                let handle = isolate.interrupt_handle();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(limit));
                    handle.timeout(limit);
                });
            }
            let mut stdout = Stdout(std::io::BufWriter::with_capacity(64 * 1024, std::io::stdout()));
            let mut context = isolate.new_context(&mut stdout);
            for (name, value) in &ini {
                context.set_ini(name, value);
            }
            // $argv[0]: the file, or php's name for stdin (-r's too).
            let argv0 = if named { STDIN_NAME.to_string() } else { path.clone() };
            context.set_args(std::iter::once(argv0).chain(args).collect());
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
    /// The script's file; none: -r's code, or the script on stdin.
    path: Option<String>,
    /// -r: code to run (without `<?php`).
    code: Option<String>,
    /// `-d name=value` settings, in order (unknown names are ignored).
    ini: Vec<(String, String)>,
    /// The script's own arguments (after the file).
    args: Vec<String>,
    /// -S: serve web requests on this address.
    serve: Option<String>,
    /// -t: the document root of -S.
    docroot: Option<String>,
    /// -q: how many times (a quieter -S log).
    quiet: u8,
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
        let (mut serve, mut docroot, mut quiet) = (None, None, 0);
        let (mut path, mut code, mut dashes) = (None, None, false);
        while let Some(arg) = args.next_if(|a| a.starts_with('-') && a != "-") {
            match arg.as_str() {
                // The script's own arguments follow (its code is on stdin
                // unless -r or -f gave it).
                "--" => {
                    dashes = true;
                    break;
                }
                "--dump" => dump = true,
                "-l" => lint = true,
                "-r" => code = Some(args.next()?),
                "-f" => path = Some(args.next()?),
                "-d" => setting(&args.next()?),
                "-S" => serve = Some(args.next()?),
                "-t" => docroot = Some(args.next()?),
                "-q" => quiet += 1,
                // No php.ini is read anyway.
                "-n" => {}
                other if other.starts_with("-d") => setting(&other[2..]),
                other if other.starts_with("-S") => serve = Some(other[2..].to_string()),
                other if other.starts_with("-t") => docroot = Some(other[2..].to_string()),
                _ => return None,
            }
        }
        // A script named after the options (-S: its router), unless -r or -f
        // gave it, or `--` said the arguments start (the script on stdin).
        if path.is_none() && code.is_none() && !dashes {
            path = args.next();
        }
        Some(Cli { dump, lint, path, code, ini, args: args.collect(), serve, docroot, quiet })
    }
}

/// What php calls a script read from stdin (its __FILE__ and $argv[0]).
const STDIN_NAME: &str = "Standard input code";

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

    fn parse(args: &[&str]) -> Option<(bool, Option<String>)> {
        Cli::parse(args.iter().map(|a| a.to_string())).map(|c| (c.dump, c.path))
    }

    fn t() -> Option<String> {
        Some("t.php".to_string())
    }

    /// What php-src's runner passes: settings, the file, the test's arguments.
    #[test]
    fn takes_the_runners_command_line() {
        let ran = parse(&["-d", "display_errors=1", "-dprecision=14", "-d", "x", "t.php", "a", "-b"]);
        assert_eq!(ran, Some((false, t())));
        let cli = Cli::parse(["-d", "error_reporting=0", "-dprecision=14", "-d", "x", "t.php"].iter().map(|a| a.to_string())).expect("parsed");
        let ini: Vec<(&str, &str)> = cli.ini.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
        assert_eq!(ini, [("error_reporting", "0"), ("precision", "14"), ("x", "1")]);
        assert_eq!(parse(&["--dump", "t.php"]), Some((true, t())));
        let lint = Cli::parse(["-l", "t.php"].iter().map(|a| a.to_string())).expect("parsed");
        assert!(lint.lint && !lint.dump && lint.path == t());
        // An unknown option or a missing value is a usage error.
        assert_eq!(parse(&["-x", "t.php"]), None);
        assert_eq!(parse(&["-d"]), None);
    }

    /// What PHPUnit runs a test in a separate process with: settings, the
    /// script on stdin, `--` and arguments; and -r, -f.
    #[test]
    fn takes_stdin_and_code() {
        let stdin = Cli::parse(["-d", "a=1", "--", "x", "y"].iter().map(|a| a.to_string())).expect("parsed");
        assert!(stdin.path.is_none() && stdin.code.is_none() && stdin.args == ["x", "y"]);
        let no_file = Cli::parse(["-d", "a=1"].iter().map(|a| a.to_string())).expect("parsed");
        assert!(no_file.path.is_none() && no_file.args.is_empty());
        let code = Cli::parse(["-r", "echo 1;", "--", "z"].iter().map(|a| a.to_string())).expect("parsed");
        assert!(code.code.as_deref() == Some("echo 1;") && code.path.is_none() && code.args == ["z"]);
        let f = Cli::parse(["-f", "t.php", "--", "a"].iter().map(|a| a.to_string())).expect("parsed");
        assert!(f.path == t() && f.args == ["a"]);
    }
}

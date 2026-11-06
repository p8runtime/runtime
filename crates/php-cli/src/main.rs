#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! The php command-line host for p8 (p8runtime/aps APS 3), named
//! php2 while p8 is built next to the legacy engine: `php2 script.php`.

use std::io::Write;

// php2's allocator, on every platform it is built for: rusty_alloc, a
// pure-Rust mimalloc (#319). Small objects come from per-thread free lists
// sharded by size, as PHP's own memory manager hands out its bins; on
// WordPress and Laravel it measured 4-7% faster than glibc and macOS's
// allocator, with the same output.
#[global_allocator]
static ALLOC: rusty_alloc_api::RustyAlloc = rusty_alloc_api::RustyAlloc;

mod http;
mod mime_types;
mod server;
mod status_phrases;

/// The extensions php2 loads (`server`: with what the built-in web server
/// adds). A WASI build has no sockets, so it is a PHP built without
/// mysqli, curl, pdo_mysql, sockets and pdo_pgsql: they are not loaded.
fn extensions(server: bool) -> Vec<p8::Extension> {
    use p8::extensions::*;
    let mut all = vec![STANDARD];
    if server {
        all.push(SERVER);
    }
    all.extend([
        XML,
        TOKENIZER,
        MBSTRING,
        PCRE,
        #[cfg(not(target_os = "wasi"))]
        MYSQLI,
        #[cfg(not(target_os = "wasi"))]
        CURL,
        GD,
        ZIP,
        FILEINFO,
        SESSION,
        PDO,
        PDO_SQLITE,
        #[cfg(not(target_os = "wasi"))]
        PDO_MYSQL,
        #[cfg(not(target_os = "wasi"))]
        SOCKETS,
        OPENSSL,
        INTL,
        #[cfg(not(target_os = "wasi"))]
        PDO_PGSQL,
    ]);
    all
}

/// `opcache.file_cache=<dir>`: keep compiled files in that directory, for
/// the runs after this one (off unless set, as in PHP). It is OPcache's,
/// so it takes effect only when OPcache starts: in the CLI (`cli`) that
/// needs opcache.enable_cli=1 too, as in PHP 8.6.
fn file_cache(platform: p8::Platform, ini: &[(String, String)], cli: bool) -> p8::Platform {
    let value = |name: &str| ini.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.trim().to_string()).or_else(|| platform.ini_default(name));
    if !p8::OpcacheSettings::from_ini(cli, &value).started() {
        return platform;
    }
    let read_only = value("opcache.file_cache_read_only").is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "on" | "yes" | "true"));
    match value("opcache.file_cache") {
        Some(dir) if !dir.is_empty() && read_only => platform.read_only_file_cache(dir),
        Some(dir) if !dir.is_empty() => platform.file_cache(dir),
        _ => platform,
    }
}

/// Zend OPcache over the kept files, started as PHP starts it from these
/// settings (`-d`, else the built-in defaults): the CLI (`cli`) needs
/// opcache.enable_cli as well as opcache.enable; php -S does not.
fn opcache(platform: p8::Platform, ini: &[(String, String)], cli: bool) -> p8::Platform {
    let value = |name: &str| ini.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.clone()).or_else(|| platform.ini_default(name));
    let settings = p8::OpcacheSettings::from_ini(cli, &value);
    platform.opcache(settings)
}

/// Run `f` with a native stack deep enough for deep PHP recursion: on a
/// thread with a 64 MiB stack. WASI has no threads; there the binary's
/// own stack is that size (see build.rs).
#[cfg(not(target_os = "wasi"))]
fn on_deep_stack(f: impl FnOnce() -> i32 + Send + 'static) -> i32 {
    std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(f).expect("interpreter thread").join().unwrap_or(255)
}

/// Run `f` with a native stack deep enough for deep PHP recursion: WASI
/// has no threads, so the binary's own stack is 64 MiB (see build.rs).
#[cfg(target_os = "wasi")]
fn on_deep_stack(f: impl FnOnce() -> i32 + Send + 'static) -> i32 {
    f()
}

fn main() {
    let Some(Cli { dump, lint, path, code, ini, args, serve, stdio, docroot, repeat, quiet, version }) = Cli::parse(std::env::args().skip(1)) else {
        eprintln!(
            "usage: php2 [--dump] [-l] [--repeat N] [-d name=value]... [-f] <file> [--] [args...]\n       php2 -v   (p8's version)\n       php2 [-d name=value]... -r <code> [--] [args...]\n       php2 [-d name=value]... [--] [args...]   (the script on stdin)\n       php2 -S <addr>:<port> [-t docroot] [-q] [router.php]\n       php2 -S <addr>:<port> --stdio <client addr>:<port> [-t docroot] [-q] [router.php]   (one request on stdin, its response on stdout)"
        );
        std::process::exit(64);
    };
    if version {
        println!("p8 {} (cli), compatible with PHP {}", p8::P8_VERSION, p8::PHP_VERSION);
        return;
    }
    // php -S: the built-in web server (the file, if any, is its router).
    if let Some(address) = serve {
        let config = server::Config { address, docroot, router: path, quiet, ini };
        let status = on_deep_stack(move || match stdio {
            Some(client) => server::run_stdio(config, client),
            None => server::run(config),
        });
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
    let platform = p8::Platform::new(&extensions(false))
        // A file included again (a template part per post) is not compiled again.
        .keep_compiled();
    let platform = file_cache(platform, &ini, true);
    let platform = opcache(platform, &ini, true);
    let file = if named { path.clone() } else { p8::canonicalize(&path).map_or(path.clone(), |p| p.to_string_lossy().into_owned()) };
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
    // Deep PHP recursion needs a deep native stack.
    let status = on_deep_stack(move || {
        let mut isolate = platform.new_isolate();
        // max_execution_time (0, the CLI's default: none): a watchdog
        // ends the run with PHP's fatal error once it has run that long.
        // (set_time_limit() calls are not followed yet.) Where there are
        // no threads (WASI) there is no watchdog: the limit is not kept,
        // as in a PHP built without timers.
        let limit = ini.iter().rev().find(|(n, _)| n == "max_execution_time").and_then(|(_, v)| v.trim().parse::<u64>().ok()).unwrap_or(0);
        if limit > 0 {
            let handle = isolate.interrupt_handle();
            let _ = std::thread::Builder::new().spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(limit));
                handle.timeout(limit);
            });
        }
        let mut stdout = Stdout(std::io::stdout());
        // $argv[0]: the file, or php's name for stdin (-r's too).
        let argv0 = if named { STDIN_NAME.to_string() } else { path.clone() };
        let argv: Vec<String> = std::iter::once(argv0).chain(args).collect();
        // --repeat N: the same script as N requests on this one isolate (a
        // warm server's view): the first one's output goes to stdout, the
        // others' nowhere; each request's time to stderr.
        let mut status = 0;
        for request in 1..=repeat {
            let started = std::time::Instant::now();
            let mut discard = Discard;
            let host: &mut dyn p8::Host = if request == 1 { &mut stdout } else { &mut discard };
            let mut context = isolate.new_context(host);
            for (name, value) in &ini {
                context.set_ini(name, value);
            }
            context.set_args(argv.clone());
            status = match platform.compile(&source, &file) {
                Ok(script) => context.run(&script),
                Err(error) => context.fail(&error),
            };
            drop(context);
            if repeat > 1 {
                eprintln!("request {request}: {:.3} ms", started.elapsed().as_secs_f64() * 1e3);
            }
        }
        status
    });
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
    /// --stdio (with -S): serve one request read from stdin, from this
    /// client address, the response on stdout (a host without sockets,
    /// such as WASI, hands the request over that way).
    stdio: Option<String>,
    /// -t: the document root of -S.
    docroot: Option<String>,
    /// --repeat N: run the script as N requests on one isolate (1: once).
    repeat: usize,
    /// -q: how many times (a quieter -S log).
    quiet: u8,
    /// -v / --version: say which p8 this is and run nothing.
    version: bool,
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
        let (mut serve, mut stdio, mut docroot, mut quiet) = (None, None, None, 0);
        let (mut path, mut code, mut dashes) = (None, None, false);
        let mut version = false;
        let mut repeat = 1;
        while let Some(arg) = args.next_if(|a| a.starts_with('-') && a != "-") {
            match arg.as_str() {
                // The script's own arguments follow (its code is on stdin
                // unless -r or -f gave it).
                "--" => {
                    dashes = true;
                    break;
                }
                "-v" | "--version" => version = true,
                "--dump" => dump = true,
                "-l" => lint = true,
                "-r" => code = Some(args.next()?),
                "-f" => path = Some(args.next()?),
                "-d" => setting(&args.next()?),
                "-S" => serve = Some(args.next()?),
                "--stdio" => stdio = Some(args.next()?),
                "-t" => docroot = Some(args.next()?),
                "-q" => quiet += 1,
                "--repeat" => repeat = args.next()?.parse().ok().filter(|&n| n > 0)?,
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
        // --stdio is a way to serve; without -S it means nothing.
        if stdio.is_some() && serve.is_none() {
            return None;
        }
        Some(Cli { dump, lint, path, code, ini, args: args.collect(), serve, stdio, docroot, repeat, quiet, version })
    }
}

/// What php calls a script read from stdin (its __FILE__ and $argv[0]).
const STDIN_NAME: &str = "Standard input code";

/// Where the output of `--repeat`'s later requests goes: nowhere.
struct Discard;

impl p8::Host for Discard {
    fn write(&mut self, _: &[u8]) {}
}

/// The terminal: a request's output goes to stdout, written through on
/// every write as PHP's CLI does (sapi_cli_ub_write writes each piece to
/// fd 1; implicit_flush is on). Nothing waits in a buffer, so output stays
/// in order with STDERR, a prompt shows before reading STDIN, and
/// fwrite(STDOUT)/fflush(STDOUT) reach the reader at once.
struct Stdout(std::io::Stdout);

impl p8::Host for Stdout {
    fn write(&mut self, bytes: &[u8]) {
        let mut out = self.0.lock();
        let _ = out.write_all(bytes);
        let _ = out.flush();
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
        // --repeat N: N requests on one isolate; N is a positive count.
        let repeated = Cli::parse(["--repeat", "5", "t.php"].iter().map(|a| a.to_string())).expect("parsed");
        assert_eq!((repeated.repeat, repeated.path.as_deref()), (5, Some("t.php")));
        assert_eq!(Cli::parse(["t.php"].iter().map(|a| a.to_string())).map(|c| c.repeat), Some(1));
        assert!(Cli::parse(["--repeat", "0", "t.php"].iter().map(|a| a.to_string())).is_none());
        assert!(Cli::parse(["--repeat", "x", "t.php"].iter().map(|a| a.to_string())).is_none());
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

    /// -S with --stdio: one request over stdin and stdout, from a client.
    #[test]
    fn version_flag() {
        assert!(Cli::parse(["-v"].iter().map(|a| a.to_string())).expect("parsed").version);
        assert!(Cli::parse(["--version"].iter().map(|a| a.to_string())).expect("parsed").version);
        assert!(!Cli::parse(["t.php"].iter().map(|a| a.to_string())).expect("parsed").version);
    }

    #[test]
    fn takes_a_request_over_stdio() {
        let cli = Cli::parse(["-S", "localhost:5173", "--stdio", "127.0.0.1:50000", "-t", "/site", "-q", "router.php"].iter().map(|a| a.to_string())).expect("parsed");
        assert_eq!(cli.serve.as_deref(), Some("localhost:5173"));
        assert_eq!(cli.stdio.as_deref(), Some("127.0.0.1:50000"));
        assert_eq!(cli.docroot.as_deref(), Some("/site"));
        assert_eq!(cli.path.as_deref(), Some("router.php"));
        // Without -S, or without the client, it is a usage error.
        assert!(Cli::parse(["--stdio", "127.0.0.1:1", "t.php"].iter().map(|a| a.to_string())).is_none());
        assert!(Cli::parse(["-S", "localhost:1", "--stdio"].iter().map(|a| a.to_string())).is_none());
    }
}

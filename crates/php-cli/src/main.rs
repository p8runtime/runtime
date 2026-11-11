#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! The php command-line host for p8 (p8runtime/aps APS 3), named
//! php2 while p8 is built next to the legacy engine: `php2 script.php`.

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

use php_cli::{Stdout, extensions, on_deep_stack};

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

/// The package a run takes compiled files from: the one `--package` names,
/// else the one appended to this executable (`--bake`), if any. One that
/// is not sound for this engine is not used, and said so on stderr.
fn open_package(platform: &p8::Platform, path: Option<&str>, check: p8::PackageCheck) -> Option<p8::Package> {
    match path {
        Some(path) => platform.open_package(path, check).map_err(|e| eprintln!("php2: not using the package {path}: {e}")).ok(),
        None => match platform.executable_package(check) {
            Ok(package) => package,
            // An executable that cannot be read back carries none.
            Err(p8::PackageError::Io(_)) => None,
            Err(e) => {
                eprintln!("php2: not using the package in this executable: {e}");
                None
            }
        },
    }
}

/// `--compile <dir>`: every PHP file under `dir` compiled into a package
/// at `out` (default `<dir>/.p8/package`), and with `bake`, a copy of this
/// executable carrying it. Returns the exit status.
fn write_package(dir: &str, out: Option<&str>, bake: Option<&str>, native: Option<&str>, native_host: Option<&str>) -> i32 {
    let platform = p8::Platform::new(&extensions(false));
    let out = out.map_or_else(|| std::path::Path::new(dir).join(".p8").join("package"), std::path::PathBuf::from);
    let packed = match platform.write_package(std::path::Path::new(dir), &out) {
        Ok(packed) => packed,
        Err(e) => {
            eprintln!("php2: could not write {}: {e}", out.display());
            return 1;
        }
    };
    for error in &packed.failed {
        eprintln!("php2: left out {} (it does not compile: {} on line {})", error.file, error.message, error.line);
    }
    println!("{} files compiled into {} ({} bytes)", packed.files, out.display(), packed.bytes);
    if let Some(exe) = bake {
        if let Err(e) = platform.bake(&out, std::path::Path::new(exe)) {
            eprintln!("php2: could not write {exe}: {e}");
            return 1;
        }
        println!("{exe}: php2 with the package");
    }
    if let Some(exe) = native {
        match php_cli::native::write(&platform, &out, native_host, exe) {
            Ok(said) => println!("{said}"),
            Err(e) => {
                eprintln!("php2: could not write {exe}: {e}");
                return 1;
            }
        }
    }
    0
}

fn main() {
    let Some(Cli { dump, lint, path, code, ini, args, serve, stdio, docroot, repeat, quiet, version, compile, package, check, bake, native, native_host }) = Cli::parse(std::env::args().skip(1)) else {
        eprintln!(
            "usage: php2 [--dump] [-l] [--repeat N] [-d name=value]... [--package <file> [--package-check hash|stat|trust]] [-f] <file> [--] [args...]\n       php2 -v   (p8's version)\n       php2 [-d name=value]... -r <code> [--] [args...]\n       php2 [-d name=value]... [--] [args...]   (the script on stdin)\n       php2 -S <addr>:<port> [-t docroot] [-q] [router.php]\n       php2 -S <addr>:<port> --stdio <client addr>:<port> [-t docroot] [-q] [router.php]   (one request on stdin, its response on stdout)\n       php2 --compile <dir> [--package <file>] [--bake <file>]   (every PHP file under <dir> compiled into a package: <dir>/.p8/package unless --package names it; --bake: also a copy of php2 that carries it)\n       php2 --compile <dir> --native <file> [--native-host <libp8_native.a>]   (also an executable with the package and its functions compiled to machine code)"
        );
        std::process::exit(64);
    };
    if version {
        println!("p8 {} (cli), compatible with PHP {}", p8::P8_VERSION, p8::PHP_VERSION);
        return;
    }
    if let Some(dir) = compile {
        std::process::exit(write_package(&dir, package.as_deref(), bake.as_deref(), native.as_deref(), native_host.as_deref()));
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
    let platform = match open_package(&platform, package.as_deref(), check) {
        Some(package) => platform.with_package(package),
        None => platform,
    };
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
    /// --compile <dir>: compile every PHP file under it into a package
    /// (written to --package, else <dir>/.p8/package) and run nothing.
    compile: Option<String>,
    /// --package <file>: take compiled files from this package (with
    /// --compile: where to write it).
    package: Option<String>,
    /// --package-check hash|stat|trust: how a run checks a packaged
    /// file against the file on disk (p8::PackageCheck).
    check: p8::PackageCheck,
    /// --bake <file> (with --compile): also write a copy of php2 with the
    /// package appended, which runs as php2 --package does.
    bake: Option<String>,
    /// --native <file> (with --compile): also write an executable with the
    /// package and its functions as machine code (`php_cli::native`).
    native: Option<String>,
    /// --native-host <file>: the runtime library it is linked with
    /// (default: libp8_native.a beside php2).
    native_host: Option<String>,
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
        let (mut compile, mut package, mut check, mut bake) = (None, None, p8::PackageCheck::Hash, None);
        let (mut native, mut native_host) = (None, None);
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
                // php has no long options of these names: `php2 compile`
                // still runs a file named compile.
                "--compile" => compile = Some(args.next()?),
                "--package" => package = Some(args.next()?),
                "--package-check" => {
                    check = match args.next()?.as_str() {
                        "hash" => p8::PackageCheck::Hash,
                        "stat" => p8::PackageCheck::Stat,
                        "trust" => p8::PackageCheck::Trust,
                        _ => return None,
                    }
                }
                "--bake" => bake = Some(args.next()?),
                "--native" => native = Some(args.next()?),
                "--native-host" => native_host = Some(args.next()?),
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
        // --stdio is a way to serve; without -S it means nothing. --bake
        // goes with --compile, and --compile runs nothing.
        let builds = bake.is_some() || native.is_some() || native_host.is_some();
        if (stdio.is_some() && serve.is_none()) || (builds && compile.is_none()) || (native_host.is_some() && native.is_none()) || (compile.is_some() && (path.is_some() || code.is_some())) {
            return None;
        }
        Some(Cli { dump, lint, path, code, ini, args: args.collect(), serve, stdio, docroot, repeat, quiet, version, compile, package, check, bake, native, native_host })
    }
}

/// What php calls a script read from stdin (its __FILE__ and $argv[0]).
const STDIN_NAME: &str = "Standard input code";

/// Where the output of `--repeat`'s later requests goes: nowhere.
struct Discard;

impl p8::Host for Discard {
    fn write(&mut self, _: &[u8]) {}
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

    /// --compile and --package are long options php does not have: a
    /// script named `compile` is still a script.
    #[test]
    fn takes_the_package_options() {
        let cli = |args: &[&str]| Cli::parse(args.iter().map(|a| a.to_string()));
        let compile = cli(&["--compile", "site", "--package", "out.p8", "--bake", "app"]).expect("parsed");
        assert_eq!((compile.compile.as_deref(), compile.package.as_deref(), compile.bake.as_deref(), compile.path), (Some("site"), Some("out.p8"), Some("app"), None));
        let run = cli(&["--package", "out.p8", "--package-check", "stat", "t.php", "a"]).expect("parsed");
        assert_eq!((run.package.as_deref(), run.check, run.path, run.args), (Some("out.p8"), p8::PackageCheck::Stat, t(), vec!["a".to_string()]));
        assert_eq!(cli(&["t.php"]).expect("parsed").check, p8::PackageCheck::Hash);
        assert_eq!(parse(&["compile", "site"]), Some((false, Some("compile".to_string()))));
        // --bake needs --compile; --compile runs no script; a check is one of three.
        assert!(cli(&["--bake", "app", "t.php"]).is_none());
        // --native goes with --compile, --native-host with --native.
        let native = cli(&["--compile", "site", "--native", "app", "--native-host", "lib.a"]).expect("parsed");
        assert_eq!((native.native.as_deref(), native.native_host.as_deref()), (Some("app"), Some("lib.a")));
        assert!(cli(&["--native", "app", "t.php"]).is_none());
        assert!(cli(&["--compile", "site", "--native-host", "lib.a"]).is_none());
        assert!(cli(&["--compile", "site", "t.php"]).is_none());
        assert!(cli(&["--package-check", "mtime", "t.php"]).is_none());
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

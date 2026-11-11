#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! `p8`, the command (p8runtime/aps APS 3, APS 8). It runs PHP as `php`
//! does, with php's command line (`p8 script.php`, `-r`, `-d`, `-S`,
//! stdin...), so Composer, PHPUnit, WP-CLI and php-src's test runner call
//! it as they call php. Two first words are its own commands (#382):
//!
//! - `p8 compile [dir]` compiles every PHP file under `dir` (default: the
//!   current directory) into its package, `<dir>/.p8/package`, which holds
//!   each file's compiled code with its source's hash. The source stays
//!   where it is.
//! - `p8 serve [dir] [--listen <addr>:<port>]` serves `dir` over HTTP with
//!   PHP's built-in web server (the one `p8 -S` runs), taking compiled
//!   files from the package: a packaged file is neither parsed nor
//!   compiled. A file changed since `p8 compile`, a new one and `eval()`'d
//!   code compile from source, as without a package; with no package (or
//!   one this p8 cannot use) the whole project does, and stderr says so.
//!   `--workers N` requests run at once, each on a worker of its own (an
//!   isolate on its own thread, sharing every compiled file: APS 8); SIGINT
//!   or SIGTERM stops it once the requests in progress have finished.
//!
//! A script named `compile` or `serve` runs as `p8 ./compile`, or after
//! any option (`p8 -f compile`).

use std::path::{Path, PathBuf};

// p8's allocator, on every platform it is built for: rusty_alloc, a
// pure-Rust mimalloc (#319). Small objects come from per-thread free lists
// sharded by size, as PHP's own memory manager hands out its bins; on
// WordPress and Laravel it measured 4-7% faster than glibc and macOS's
// allocator, with the same output.
#[global_allocator]
static ALLOC: rusty_alloc_api::RustyAlloc = rusty_alloc_api::RustyAlloc;

mod http;
mod mime_types;
mod pool;
mod server;
mod status_phrases;

const USAGE: &str = "usage: p8 [--dump] [-l] [--repeat N] [-d name=value]... [--package <file> [--package-check hash|stat|trust]] [-f] <file> [--] [args...]
       p8 -v   (p8's version)
       p8 [-d name=value]... -r <code> [--] [args...]
       p8 [-d name=value]... [--] [args...]   (the script on stdin)
       p8 -S <addr>:<port> [-t docroot] [-q] [router.php]
       p8 -S <addr>:<port> --stdio <client addr>:<port> [-t docroot] [-q] [router.php]   (one request on stdin, its response on stdout)
       p8 compile [dir] [--package <file>] [--cli] [--bake <file>]   (every PHP file under dir, default the current directory, compiled into dir/.p8/package, for p8 serve; --package: written there instead; --cli: for `p8 --package <file> <script>` runs instead; --bake: also a copy of p8 that runs scripts with it, implies --cli)
       p8 compile [dir] --native <file> [--native-host <libp8_native.a>]   (also an executable with the package and its functions as machine code; implies --cli)
       p8 serve [dir] [--listen <addr>:<port>] [--workers N] [-d name=value]...   (dir, default the current directory, served over HTTP from dir/.p8/package; default 127.0.0.1:8000, two workers per core)";

// (Shared with the native executables `p8 compile --native` builds.)
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
        Some(path) => platform.open_package(path, check).map_err(|e| eprintln!("p8: not using the package {path}: {e}")).ok(),
        None => match platform.executable_package(check) {
            Ok(package) => package,
            // An executable that cannot be read back carries none.
            Err(p8::PackageError::Io(_)) => None,
            Err(e) => {
                eprintln!("p8: not using the package in this executable: {e}");
                None
            }
        },
    }
}

/// A project's package: `<dir>/.p8/package`.
fn package_of(dir: &str) -> PathBuf {
    Path::new(dir).join(".p8").join("package")
}

/// `p8 compile`: every PHP file under the project's folder compiled into a
/// package, for the engine that will use it (a package is used only by the
/// engine, extensions included, that wrote it): `p8 serve`'s, or with
/// `--cli` the one that runs scripts. With `--bake`, also a copy of this
/// executable carrying it. Returns the exit status.
fn write_package(compile: &Compile) -> i32 {
    let platform = p8::Platform::new(&extensions(!compile.cli));
    let out = compile.package.as_ref().map_or_else(|| package_of(&compile.dir), PathBuf::from);
    let packed = match platform.write_package(Path::new(&compile.dir), &out) {
        Ok(packed) => packed,
        Err(e) => {
            eprintln!("p8: could not write {}: {e}", out.display());
            return 1;
        }
    };
    for error in &packed.failed {
        eprintln!("p8: left out {} (it does not compile: {} on line {})", error.file, error.message, error.line);
    }
    println!("{} files compiled into {} ({} bytes)", packed.files, out.display(), packed.bytes);
    if let Some(exe) = &compile.bake {
        if let Err(e) = platform.bake(&out, Path::new(exe)) {
            eprintln!("p8: could not write {exe}: {e}");
            return 1;
        }
        println!("{exe}: p8 with the package");
    }
    if let Some(exe) = &compile.native {
        match php_cli::native::write(&platform, &out, compile.native_host.as_deref(), exe) {
            Ok(said) => println!("{said}"),
            Err(e) => {
                eprintln!("p8: could not write {exe}: {e}");
                return 1;
            }
        }
    }
    0
}

fn main() {
    let Some(command) = Command::parse(std::env::args().skip(1)) else {
        eprintln!("{USAGE}");
        std::process::exit(64);
    };
    let status = match command {
        Command::Compile(compile) => write_package(&compile),
        Command::Serve(Serve { dir, listen, workers, ini }) => {
            let project = server::Project { package: package_of(&dir) };
            let workers = workers.unwrap_or_else(default_workers);
            let config = server::Config { address: listen, docroot: Some(dir), router: None, quiet: 0, ini, project: Some(project), workers };
            on_deep_stack(move || server::run(config))
        }
        Command::Php(cli) => php(cli),
    };
    std::process::exit(status);
}

/// What php's command line asks for: the script run, `-S` served, `-l`
/// or `--dump`. Returns the exit status.
fn php(cli: Cli) -> i32 {
    let Cli { dump, lint, path, code, ini, args, serve, stdio, docroot, repeat, quiet, version, package, check } = cli;
    if version {
        println!("p8 {} (cli), compatible with PHP {}", p8::P8_VERSION, p8::PHP_VERSION);
        return 0;
    }
    // php -S: the built-in web server (the file, if any, is its router).
    if let Some(address) = serve {
        let config = server::Config { address, docroot, router: path, quiet, ini, project: None, workers: 1 };
        return on_deep_stack(move || match stdio {
            Some(client) => server::run_stdio(config, client),
            None => server::run(config),
        });
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
                return 1;
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
        return match platform.compile(&source, &path) {
            Ok(_) => {
                println!("No syntax errors detected in {path}");
                0
            }
            Err(error) => {
                let label = if error.kind == p8::ErrorKind::Parse { "Parse error" } else { "Fatal error" };
                println!("\n{label}: {} in {path} on line {}\nErrors parsing {path}", error.message, error.line);
                255
            }
        };
    }
    if dump {
        return match platform.compile(&source, &file) {
            Ok(script) => {
                print!("{}", platform.disassemble(&script));
                0
            }
            Err(error) => {
                eprintln!("{}", error.message);
                255
            }
        };
    }
    // Deep PHP recursion needs a deep native stack.
    on_deep_stack(move || {
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
    })
}

/// What the command line asks for: one of p8's own commands (its first
/// word), else what php's command line asks for.
#[derive(Debug, PartialEq)]
enum Command {
    Compile(Compile),
    Serve(Serve),
    Php(Cli),
}

/// `p8 serve [dir] [--listen <addr>:<port>] [--workers N] [-d name=value]...`.
#[derive(Debug, PartialEq)]
struct Serve {
    dir: String,
    listen: String,
    /// Requests run at once (default: [`default_workers`]).
    workers: Option<usize>,
    /// `-d name=value` settings for every request, in order.
    ini: Vec<(String, String)>,
}

/// How many workers `p8 serve` runs unless `--workers` says: two per core.
/// A PHP request blocks its thread while it waits (a database, a file, a
/// remote call), so more workers than cores keep the cores busy, as
/// php-fpm runs more children than cores (APS 8: its worker count, as
/// threads); an idle worker's isolate holds no heap of its own.
fn default_workers() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get()).saturating_mul(2).max(2)
}

/// `name=value` (a bare name is `name=1`), as php's `-d` takes it.
fn setting(text: &str) -> (String, String) {
    let (name, value) = text.split_once('=').unwrap_or((text, "1"));
    (name.trim().to_string(), value.trim().trim_matches('"').to_string())
}

/// `p8 compile [dir] [--package <file>] [--cli] [--bake <file>]`.
#[derive(Debug, PartialEq)]
struct Compile {
    dir: String,
    /// Where the package goes (default `<dir>/.p8/package`).
    package: Option<String>,
    /// For runs of scripts (`p8 --package <file> script.php`), not for
    /// `p8 serve`: the two load different extensions.
    cli: bool,
    /// Also write a copy of p8 with the package appended, which runs
    /// scripts as `p8 --package` does (so `--cli` is implied).
    bake: Option<String>,
    /// `--native <file>`: also write an executable with the package and its
    /// functions as machine code (`php_cli::native`; `--cli` is implied).
    native: Option<String>,
    /// `--native-host <file>`: the runtime library it is linked with
    /// (default: libp8_native.a beside p8).
    native_host: Option<String>,
}

/// Where `p8 serve` listens unless `--listen` says otherwise.
const LISTEN: &str = "127.0.0.1:8000";

impl Command {
    fn parse(args: impl Iterator<Item = String>) -> Option<Command> {
        let mut args = args.peekable();
        match args.peek().map(String::as_str) {
            Some("compile") => {
                args.next();
                let (mut dir, mut package, mut cli, mut bake) = (None, None, false, None);
                let (mut native, mut native_host) = (None, None);
                while let Some(arg) = args.next() {
                    match arg.as_str() {
                        "--package" if package.is_none() => package = Some(args.next()?),
                        "--cli" => cli = true,
                        "--bake" if bake.is_none() => bake = Some(args.next()?),
                        "--native" if native.is_none() => native = Some(args.next()?),
                        "--native-host" if native_host.is_none() => native_host = Some(args.next()?),
                        _ if !arg.starts_with('-') && dir.is_none() => dir = Some(arg),
                        _ => return None,
                    }
                }
                if native_host.is_some() && native.is_none() {
                    return None;
                }
                let cli = cli || bake.is_some() || native.is_some();
                Some(Command::Compile(Compile { dir: dir.unwrap_or_else(|| ".".to_string()), package, cli, bake, native, native_host }))
            }
            Some("serve") => {
                args.next();
                let (mut dir, mut listen, mut workers, mut ini) = (None, None, None, Vec::new());
                while let Some(arg) = args.next() {
                    match arg.as_str() {
                        "--listen" if listen.is_none() => listen = Some(args.next()?),
                        "--workers" if workers.is_none() => workers = Some(args.next()?.parse().ok().filter(|&n: &usize| n > 0)?),
                        "-d" => ini.push(setting(&args.next()?)),
                        other if other.starts_with("-d") && other.len() > 2 => ini.push(setting(&other[2..])),
                        _ if !arg.starts_with('-') && dir.is_none() => dir = Some(arg),
                        _ => return None,
                    }
                }
                Some(Command::Serve(Serve { dir: dir.unwrap_or_else(|| ".".to_string()), listen: listen.unwrap_or_else(|| LISTEN.to_string()), workers, ini }))
            }
            _ => Cli::parse(args).map(Command::Php),
        }
    }
}

/// php's command line: options, the file, then the script's own arguments.
#[derive(Debug, PartialEq)]
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
    /// --package <file>: take compiled files from this package (one
    /// `p8 compile --cli` wrote).
    package: Option<String>,
    /// --package-check hash|stat|trust: how a run checks a packaged
    /// file against the file on disk (p8::PackageCheck).
    check: p8::PackageCheck,
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
        let (mut package, mut check) = (None, p8::PackageCheck::Hash);
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
                // php has no long options of these names.
                "--package" => package = Some(args.next()?),
                "--package-check" => {
                    check = match args.next()?.as_str() {
                        "hash" => p8::PackageCheck::Hash,
                        "stat" => p8::PackageCheck::Stat,
                        "trust" => p8::PackageCheck::Trust,
                        _ => return None,
                    }
                }
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
        Some(Cli { dump, lint, path, code, ini, args: args.collect(), serve, stdio, docroot, repeat, quiet, version, package, check })
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
    use super::{Cli, Command, Compile, LISTEN, Serve};

    fn parse(args: &[&str]) -> Option<(bool, Option<String>)> {
        Cli::parse(args.iter().map(|a| a.to_string())).map(|c| (c.dump, c.path))
    }

    fn command(args: &[&str]) -> Option<Command> {
        Command::parse(args.iter().map(|a| a.to_string()))
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

    /// --package and --package-check are long options php does not have.
    #[test]
    fn takes_the_package_options() {
        let cli = |args: &[&str]| Cli::parse(args.iter().map(|a| a.to_string()));
        let run = cli(&["--package", "out.p8", "--package-check", "stat", "t.php", "a"]).expect("parsed");
        assert_eq!((run.package.as_deref(), run.check, run.path, run.args), (Some("out.p8"), p8::PackageCheck::Stat, t(), vec!["a".to_string()]));
        assert_eq!(cli(&["t.php"]).expect("parsed").check, p8::PackageCheck::Hash);
        assert!(cli(&["--package-check", "mtime", "t.php"]).is_none());
    }

    /// `compile` and `serve` are p8's own commands only as the first word:
    /// a script of that name runs as `./compile`, or after an option.
    #[test]
    fn takes_compile_and_serve() {
        let compile = |dir: &str, package: Option<&str>, cli: bool, bake: Option<&str>| {
            Some(Command::Compile(Compile { dir: dir.into(), package: package.map(Into::into), cli, bake: bake.map(Into::into), native: None, native_host: None }))
        };
        assert_eq!(command(&["compile"]), compile(".", None, false, None));
        assert_eq!(command(&["compile", "site", "--package", "out.p8"]), compile("site", Some("out.p8"), false, None));
        assert_eq!(command(&["compile", "--cli", "site"]), compile("site", None, true, None));
        assert_eq!(command(&["compile", "site", "--bake", "app"]), compile("site", None, true, Some("app")));
        // --native implies --cli; --native-host goes with --native.
        let native = Compile { dir: "site".into(), package: None, cli: true, bake: None, native: Some("app".into()), native_host: Some("lib.a".into()) };
        assert_eq!(command(&["compile", "site", "--native", "app", "--native-host", "lib.a"]), Some(Command::Compile(native)));
        assert_eq!(command(&["compile", "site", "--native-host", "lib.a"]), None);
        let serve = |dir: &str, listen: &str, workers: Option<usize>, ini: &[(&str, &str)]| {
            Some(Command::Serve(Serve { dir: dir.into(), listen: listen.into(), workers, ini: ini.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect() }))
        };
        assert_eq!(command(&["serve"]), serve(".", LISTEN, None, &[]));
        assert_eq!(command(&["serve", "site", "--listen", "0.0.0.0:80"]), serve("site", "0.0.0.0:80", None, &[]));
        assert_eq!(command(&["serve", "--listen", "[::1]:9000", "site"]), serve("site", "[::1]:9000", None, &[]));
        assert_eq!(command(&["serve", "site", "--workers", "3", "-d", "max_execution_time=1", "-dprecision=5"]), serve("site", LISTEN, Some(3), &[("max_execution_time", "1"), ("precision", "5")]));
        assert!(super::default_workers() >= 2);
        for wrong in [
            &["compile", "a", "b"][..],
            &["compile", "--listen", "x:1"],
            &["compile", "--package"],
            &["serve", "--listen"],
            &["serve", "-x"],
            &["serve", "a", "b"],
            &["serve", "--workers", "0"],
            &["serve", "--workers", "x"],
            &["serve", "-d"],
        ] {
            assert_eq!(command(wrong), None, "{wrong:?}");
        }
        // A script named compile or serve.
        for (args, script) in [(&["./compile", "x"][..], "./compile"), (&["-f", "serve"], "serve"), (&["-d", "a=1", "compile"], "compile")] {
            match command(args) {
                Some(Command::Php(cli)) => assert_eq!(cli.path.as_deref(), Some(script), "{args:?}"),
                other => panic!("{args:?}: {other:?}"),
            }
        }
        // Anything else is php's command line.
        assert!(matches!(command(&["t.php", "compile"]), Some(Command::Php(cli)) if cli.path == t() && cli.args == ["compile"]));
    }

    #[test]
    fn version_flag() {
        assert!(Cli::parse(["-v"].iter().map(|a| a.to_string())).expect("parsed").version);
        assert!(Cli::parse(["--version"].iter().map(|a| a.to_string())).expect("parsed").version);
        assert!(!Cli::parse(["t.php"].iter().map(|a| a.to_string())).expect("parsed").version);
    }

    /// -S with --stdio: one request over stdin and stdout, from a client.
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

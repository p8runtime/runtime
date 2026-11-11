#![forbid(unsafe_code)] // memory safety is the point of p8; see SECURITY.md
//! `p8`: a PHP project compiled ahead and served (#382, APS 8).
//!
//! - `p8 compile [dir]` compiles every PHP file under `dir` (default: the
//!   current directory) into its package, `<dir>/.p8/package`, which holds
//!   each file's compiled code with its source's hash. The source stays
//!   where it is.
//! - `p8 serve [dir] [--listen <addr>:<port>]` serves `dir` over HTTP with
//!   PHP's built-in web server (the one `php2 -S` runs), taking compiled
//!   files from the package: a packaged file is neither parsed nor
//!   compiled. A file changed since `p8 compile`, a new one and `eval()`'d
//!   code compile from source, as without a package; with no package (or
//!   one this p8 cannot use) the whole project does, and stderr says so.

// The allocator php2 runs with (see main.rs).
#[global_allocator]
static ALLOC: rusty_alloc_api::RustyAlloc = rusty_alloc_api::RustyAlloc;

use std::path::{Path, PathBuf};

const USAGE: &str = "usage: p8 compile [dir]   (every PHP file under dir, default the current directory, compiled into dir/.p8/package)
       p8 serve [dir] [--listen <addr>:<port>]   (dir served over HTTP from its package; default 127.0.0.1:8000)
       p8 --version";

/// Where `serve` listens unless `--listen` says otherwise.
const LISTEN: &str = "127.0.0.1:8000";

/// What the command line asks for.
#[derive(Debug, PartialEq)]
enum Command {
    Compile { dir: String },
    Serve { dir: String, listen: String },
    Version,
}

impl Command {
    fn parse(args: impl Iterator<Item = String>) -> Option<Command> {
        let mut args = args.peekable();
        let command = args.next()?;
        let mut dir = None;
        let mut listen = None;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--listen" if command == "serve" && listen.is_none() => listen = Some(args.next()?),
                other if !other.starts_with('-') && dir.is_none() => dir = Some(arg),
                _ => return None,
            }
        }
        let dir = dir.unwrap_or_else(|| ".".to_string());
        match command.as_str() {
            "compile" => Some(Command::Compile { dir }),
            "serve" => Some(Command::Serve { dir, listen: listen.unwrap_or_else(|| LISTEN.to_string()) }),
            "-v" | "--version" if args.peek().is_none() && dir == "." => Some(Command::Version),
            _ => None,
        }
    }
}

/// A project's package: `<dir>/.p8/package`.
fn package_of(dir: &str) -> PathBuf {
    Path::new(dir).join(".p8").join("package")
}

fn main() {
    let Some(command) = Command::parse(std::env::args().skip(1)) else {
        eprintln!("{USAGE}");
        std::process::exit(64);
    };
    let status = match command {
        Command::Version => {
            println!("p8 {}, compatible with PHP {}", p8::P8_VERSION, p8::PHP_VERSION);
            0
        }
        // Compiled for the server's engine: a package is used only by the
        // engine (extensions included) that wrote it.
        Command::Compile { dir } => php_cli::write_package("p8", true, &dir, &package_of(&dir), None),
        Command::Serve { dir, listen } => {
            let project = php_cli::server::Project { package: package_of(&dir) };
            let config = php_cli::server::Config { address: listen, docroot: Some(dir), router: None, quiet: 0, ini: Vec::new(), project: Some(project) };
            php_cli::on_deep_stack(move || php_cli::server::run(config))
        }
    };
    std::process::exit(status);
}

#[cfg(test)]
mod tests {
    use super::{Command, LISTEN};

    fn parse(args: &[&str]) -> Option<Command> {
        Command::parse(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn takes_compile_and_serve() {
        assert_eq!(parse(&["compile"]), Some(Command::Compile { dir: ".".into() }));
        assert_eq!(parse(&["compile", "site"]), Some(Command::Compile { dir: "site".into() }));
        assert_eq!(parse(&["serve"]), Some(Command::Serve { dir: ".".into(), listen: LISTEN.into() }));
        assert_eq!(parse(&["serve", "site", "--listen", "0.0.0.0:80"]), Some(Command::Serve { dir: "site".into(), listen: "0.0.0.0:80".into() }));
        assert_eq!(parse(&["serve", "--listen", "[::1]:9000", "site"]), Some(Command::Serve { dir: "site".into(), listen: "[::1]:9000".into() }));
        assert_eq!(parse(&["--version"]), Some(Command::Version));
        // Anything else is a usage error.
        for wrong in [&[][..], &["run"], &["compile", "a", "b"], &["compile", "--listen", "x:1"], &["serve", "--listen"], &["serve", "-x"], &["--version", "x"]] {
            assert_eq!(parse(wrong), None, "{wrong:?}");
        }
    }
}

//! `p8 compile` then `p8 serve` (#382): a request is answered with what
//! the CLI prints for the same script, and compiles nothing (each request's
//! log line counts the files it compiled from source); a file edited or
//! added after `p8 compile` is served with its new code; with no `.p8/`,
//! or a package another engine wrote, the project is still served, from
//! source, and stderr says so once.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A project: index.php includes lib.php and every plugins/*.php.
fn project(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("p8_serve").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("plugins")).expect("test dir");
    std::fs::write(
        dir.join("index.php"),
        "<?php require __DIR__ . '/lib.php';\nforeach (glob(__DIR__ . '/plugins/*.php') as $p) { include $p; }\necho greet('p8'), ' ', eval('return 6 * 7;'), \"\\n\";\n",
    )
    .expect("index.php");
    std::fs::write(dir.join("lib.php"), "<?php function greet($who) { return \"hello $who\"; }\n").expect("lib.php");
    std::fs::write(dir.join("plugins/one.php"), "<?php echo 'one ';").expect("one.php");
    dir.canonicalize().expect("canonical")
}

fn run(exe: &str, args: &[&str]) -> Output {
    Command::new(exe).args(args).stdin(Stdio::null()).output().expect("run")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// What `php2 <dir>/index.php` prints.
fn cli_output(dir: &Path) -> String {
    let out = run(env!("CARGO_BIN_EXE_php2"), &[dir.join("index.php").to_str().expect("utf-8")]);
    assert!(out.status.success() && out.stderr.is_empty(), "{}", text(&out.stderr));
    text(&out.stdout)
}

fn compile(dir: &Path) {
    let out = run(env!("CARGO_BIN_EXE_p8"), &["compile", dir.to_str().expect("utf-8")]);
    let stdout = text(&out.stdout);
    assert!(out.status.success() && stdout.starts_with("3 files compiled into ") && stdout.contains(".p8/package"), "{stdout}{}", text(&out.stderr));
}

/// A running `p8 serve` and what it has logged.
struct Serve {
    child: Child,
    port: u16,
    log: Arc<Mutex<Vec<String>>>,
}

impl Serve {
    fn start(dir: &Path) -> Serve {
        let mut child = Command::new(env!("CARGO_BIN_EXE_p8"))
            .args(["serve", dir.to_str().expect("utf-8"), "--listen", "127.0.0.1:0"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start p8 serve");
        let stderr = child.stderr.take().expect("stderr");
        let log = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = std::sync::mpsc::channel();
        let shared = Arc::clone(&log);
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(rest) = line.split(" at http://127.0.0.1:").nth(1) {
                    let _ = tx.send(rest.trim().parse::<u16>().unwrap_or(0));
                }
                shared.lock().expect("log").push(line);
            }
        });
        let port = rx.recv_timeout(Duration::from_secs(20)).expect("the server's serving line");
        Serve { child, port, log }
    }

    /// The body of the response to GET `path`, with the request's log
    /// line (which says how many files it compiled from source).
    fn get(&self, path: &str) -> (String, String) {
        let seen = self.log.lock().expect("log").len();
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(20))).expect("timeout");
        write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").expect("send");
        let mut response = Vec::new();
        stream.read_to_end(&mut response).expect("response");
        let response = text(&response);
        let (head, body) = response.split_once("\r\n\r\n").expect("a head and a body");
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
        let wanted = format!("[200]: GET {path} - ");
        let started = Instant::now();
        loop {
            if let Some(line) = self.log.lock().expect("log")[seen..].iter().find(|l| l.contains(&wanted)) {
                return (body.to_string(), line.clone());
            }
            assert!(started.elapsed() < Duration::from_secs(20), "no log line for {path}: {:?}", self.log());
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn log(&self) -> Vec<String> {
        self.log.lock().expect("log").clone()
    }

    /// The lines that are not the date-stamped server log (what p8 says
    /// about the package).
    fn notes(&self) -> Vec<String> {
        self.log().into_iter().filter(|l| !l.starts_with('[')).collect()
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn a_compiled_project_is_served_from_its_package() {
    let dir = project("served");
    let expected = cli_output(&dir);
    assert_eq!(expected, "one hello p8 42\n");
    compile(&dir);
    let serve = Serve::start(&dir);
    // The package is used: nothing to say about it, and nothing compiled.
    assert_eq!(serve.notes(), Vec::<String>::new());
    let (body, line) = serve.get("/index.php");
    assert_eq!(body, expected);
    assert!(line.ends_with(" - 0 files compiled from source"), "{line}");
    // Served again: still nothing compiled.
    let (body, line) = serve.get("/");
    assert_eq!(body, expected);
    assert!(line.ends_with(" - 0 files compiled from source"), "{line}");
    // An edit after `p8 compile`: the next request runs the new code (that
    // one file compiled from source).
    std::fs::write(dir.join("lib.php"), "<?php function greet($who) { return \"howdy $who\"; }\n").expect("edit");
    let (body, line) = serve.get("/index.php");
    assert_eq!(body, "one howdy p8 42\n");
    assert!(line.ends_with(" - 1 file compiled from source"), "{line}");
    // A new file is served too.
    std::fs::write(dir.join("plugins/two.php"), "<?php echo 'two ';").expect("two.php");
    let (body, line) = serve.get("/index.php");
    assert_eq!(body, "one two howdy p8 42\n");
    assert!(line.ends_with(" - 1 file compiled from source"), "{line}");
    assert_eq!(body, cli_output(&dir));
}

#[test]
fn a_project_without_a_package_is_served_from_source() {
    let dir = project("unpackaged");
    compile(&dir);
    std::fs::remove_dir_all(dir.join(".p8")).expect("remove .p8");
    let serve = Serve::start(&dir);
    let (body, line) = serve.get("/index.php");
    assert_eq!(body, cli_output(&dir));
    assert!(line.ends_with(" - 3 files compiled from source"), "{line}");
    let package = dir.join(".p8/package");
    assert_eq!(
        serve.notes(),
        [format!("p8: there is no compiled package at {}, so every file is compiled from source (`p8 compile` writes it).", package.display())]
    );
}

#[test]
fn a_package_for_another_engine_is_not_used() {
    // php2 --compile writes a package for the CLI's extensions; the server
    // loads more (the built-in server's), so it is another engine's.
    let dir = project("cli_package");
    let out = run(env!("CARGO_BIN_EXE_php2"), &["--compile", dir.to_str().expect("utf-8")]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let serve = Serve::start(&dir);
    let (body, line) = serve.get("/index.php");
    assert_eq!(body, cli_output(&dir));
    assert!(line.ends_with(" - 3 files compiled from source"), "{line}");
    let package = dir.join(".p8/package");
    assert_eq!(
        serve.notes(),
        [format!(
            "p8: the package at {} was written by another build of p8 or for other extensions, so every file is compiled from source (`p8 compile` writes it again).",
            package.display()
        )]
    );
}

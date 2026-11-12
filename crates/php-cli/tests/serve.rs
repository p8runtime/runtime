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

/// What `p8 <dir>/index.php` prints.
fn cli_output(dir: &Path) -> String {
    let out = run(env!("CARGO_BIN_EXE_p8"),&[dir.join("index.php").to_str().expect("utf-8")]);
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
        Serve::start_with(dir, &[])
    }

    /// `p8 serve <dir>` on a free port, with options `extra`.
    fn start_with(dir: &Path, extra: &[&str]) -> Serve {
        let mut command = Command::new(env!("CARGO_BIN_EXE_p8"));
        command.args(["serve", dir.to_str().expect("utf-8"), "--listen", "127.0.0.1:0"]).args(extra);
        Serve::spawn(command)
    }

    /// `command` (a `p8 serve` or a `p8 -S` on port 0) started; its port
    /// read from the line the server logs when it listens.
    fn spawn(mut command: Command) -> Serve {
        let mut child = command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().expect("start the server");
        let stderr = child.stderr.take().expect("stderr");
        let log = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = std::sync::mpsc::channel();
        let shared = Arc::clone(&log);
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                // `p8 serve`: "… at http://127.0.0.1:PORT"; `p8 -S`: "… (http://127.0.0.1:PORT) started".
                let port = line.split(" at http://127.0.0.1:").nth(1).or_else(|| line.split("(http://127.0.0.1:").nth(1)).and_then(|rest| rest.trim().trim_end_matches(") started").parse::<u16>().ok());
                if let Some(port) = port {
                    let _ = tx.send(port);
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

    /// Wait (at most 20 s) until the log has `n` lines containing `wanted`;
    /// those lines.
    fn wait_for(&self, wanted: &str, n: usize) -> Vec<String> {
        let started = Instant::now();
        loop {
            let found: Vec<String> = self.log().into_iter().filter(|l| l.contains(wanted)).collect();
            if found.len() >= n {
                return found;
            }
            assert!(started.elapsed() < Duration::from_secs(20), "{n} log lines with {wanted:?} expected: {:?}", self.log());
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// The lines that are not the date-stamped server log (what p8 says
    /// about the package).
    fn notes(&self) -> Vec<String> {
        self.log().into_iter().filter(|l| !l.starts_with('[')).collect()
    }
}

/// The head and the body of the response to GET `path` on `port`, and
/// when it had come in whole.
fn fetch(port: u16, path: &str) -> (String, String, Instant) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream.set_read_timeout(Some(Duration::from_secs(20))).expect("timeout");
    write!(stream, "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").expect("send");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).expect("a whole response within 20 s");
    let at = Instant::now();
    let response = text(&response);
    let (head, body) = response.split_once("\r\n\r\n").unwrap_or((&response, ""));
    (head.to_string(), body.to_string(), at)
}

/// The number of files a request's log line says it compiled from source.
fn compiled(line: &str) -> usize {
    let count = line.rsplit(" - ").next().and_then(|s| s.split(' ').next()).expect("a count");
    count.parse().unwrap_or_else(|_| panic!("a count in {line:?}"))
}

/// A project of its own (`name`) holding `files` (name, source).
fn site(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("p8_serve").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("test dir");
    for (file, source) in files {
        std::fs::write(dir.join(file), source).expect("write");
    }
    dir.canonicalize().expect("canonical")
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
    // `p8 compile --cli` writes a package for the CLI's extensions; the
    // server loads more (the built-in server's), so it is another engine's.
    let dir = project("cli_package");
    let out = run(env!("CARGO_BIN_EXE_p8"), &["compile", dir.to_str().expect("utf-8"), "--cli"]);
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

/// `--workers 2`: a request that sleeps does not hold up one sent while it
/// runs; the second is answered first.
#[test]
fn requests_run_side_by_side() {
    let dir = site("side_by_side", &[("slow.php", "<?php sleep(2); echo 'slow';"), ("fast.php", "<?php echo 'fast';")]);
    let serve = Serve::start_with(&dir, &["--workers", "2"]);
    let port = serve.port;
    let slow = std::thread::spawn(move || fetch(port, "/slow.php"));
    std::thread::sleep(Duration::from_millis(300));
    let (_, fast_body, fast_at) = fetch(port, "/fast.php");
    let (_, slow_body, slow_at) = slow.join().expect("the slow request");
    assert_eq!((slow_body.as_str(), fast_body.as_str()), ("slow", "fast"));
    assert!(fast_at < slow_at, "the fast request was answered after the slow one: it waited for it");
}

/// Workers share one engine: three requests running at once on three
/// workers, each the first to need the project's three files, compile
/// each file once between them (one worker compiles it, the others wait
/// and take it).
#[test]
fn every_worker_runs_the_code_compiled_once() {
    let dir = project("shared");
    std::fs::write(dir.join("lib.php"), "<?php function greet($who) { usleep(500000); return \"hello $who\"; }\n").expect("lib.php");
    let serve = Serve::start_with(&dir, &["--workers", "3"]);
    let port = serve.port;
    let requests: Vec<_> = (0..3).map(|_| std::thread::spawn(move || fetch(port, "/index.php"))).collect();
    for request in requests {
        let (head, body, _) = request.join().expect("a request");
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
        assert_eq!(body, "one hello p8 42\n");
    }
    let lines = serve.wait_for("[200]: GET /index.php - ", 3);
    assert_eq!(lines.iter().map(|l| compiled(l)).sum::<usize>(), 3, "{lines:?}");
}

/// What one request leaves (globals, constants, functions, classes and
/// their statics, a function's statics, ini_set(), an open output buffer,
/// its session) is not seen by the next, on its worker or another.
#[test]
fn a_request_leaves_nothing_to_the_next() {
    let lib = "<?php function counter() { static $n = 0; return ++$n; }\n";
    let leak = "<?php require __DIR__ . '/lib.php';
$GLOBALS['leaked'] = 1; define('LEAKED', 1); function leaked_fn() {} class LeakedClass { public static $n = 0; } LeakedClass::$n = 5;
counter(); counter(); ini_set('precision', '3'); session_start(); $_SESSION['secret'] = 'leak';
ob_start(); echo 'left open'; usleep(500000);";
    let check = "<?php require __DIR__ . '/lib.php'; session_start();
echo isset($GLOBALS['leaked']) ? 1 : 0, defined('LEAKED') ? 1 : 0, function_exists('leaked_fn') ? 1 : 0, class_exists('LeakedClass') ? 1 : 0,
' ', ini_get('precision'), ' ', ob_get_level(), ' ', $_SESSION['secret'] ?? 'none', ' ', counter();";
    let dir = site("isolated", &[("lib.php", lib), ("leak.php", leak), ("check.php", check)]);
    let serve = Serve::start_with(&dir, &["--workers", "2"]);
    let port = serve.port;
    // What a request sees on a server nothing ran on yet.
    let fresh = fetch(port, "/check.php").1;
    assert!(fresh.starts_with("0000 14 ") && fresh.ends_with(" none 1"), "{fresh}");
    // Both at once: each worker runs one.
    let leaks: Vec<_> = (0..2).map(|_| std::thread::spawn(move || fetch(port, "/leak.php"))).collect();
    for leak in leaks {
        assert_eq!(leak.join().expect("a leaking request").1, "left open");
    }
    for _ in 0..4 {
        assert_eq!(fetch(port, "/check.php").1, fresh);
    }
}

/// A request past max_execution_time ends with PHP's fatal error, and its
/// worker (the only one) serves the next request; so does a request that
/// calls exit().
#[test]
fn a_request_past_its_time_ends_and_its_worker_goes_on() {
    let dir = site("time_limit", &[("loop.php", "<?php echo 'looping '; while (true) {}"), ("exit.php", "<?php echo 'bye'; exit(3);"), ("fast.php", "<?php echo 'fast';")]);
    let serve = Serve::start_with(&dir, &["--workers", "1", "-d", "max_execution_time=1"]);
    let started = Instant::now();
    let (_, body, _) = fetch(serve.port, "/loop.php");
    assert!(body.contains("Maximum execution time of 1 second exceeded"), "{body}");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert_eq!(fetch(serve.port, "/fast.php").1, "fast");
    assert_eq!(fetch(serve.port, "/exit.php").1, "bye");
    assert_eq!(fetch(serve.port, "/fast.php").1, "fast");
}

/// A recursion that never ends is stopped by memory_limit, and the worker
/// (the only one) then runs the next request as a fresh one would: the
/// register file the recursion grew is not kept against the next request,
/// so a small request under a small limit (8M) runs.
#[test]
fn a_runaway_recursion_leaves_its_worker_serving() {
    let forever = "<?php function forever($n) { return forever($n + 1) + 1; } echo forever(0);";
    // (100 calls deep: the call stack grows, so memory_limit is checked.)
    let fast = "<?php ini_set('memory_limit', '8M'); function down($n) { return $n ? down($n - 1) + 1 : 0; } echo 'fast ', down(100);";
    let dir = site("runaway", &[("forever.php", forever), ("fast.php", fast)]);
    let serve = Serve::start_with(&dir, &["--workers", "1"]);
    let started = Instant::now();
    let _ = fetch(serve.port, "/forever.php");
    assert!(started.elapsed() < Duration::from_secs(15), "the runaway request took {:?}", started.elapsed());
    for _ in 0..2 {
        let (head, body, _) = fetch(serve.port, "/fast.php");
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n") && body == "fast 100", "{head}{body}");
    }
}

/// SIGTERM: the request in progress is answered in full, no new one is
/// taken, and the server exits with status 0.
#[test]
fn sigterm_lets_requests_in_progress_finish() {
    let dir = site("shutdown", &[("slow.php", "<?php sleep(2); echo 'slow';"), ("fast.php", "<?php echo 'fast';")]);
    let mut serve = Serve::start_with(&dir, &["--workers", "2"]);
    let port = serve.port;
    let slow = std::thread::spawn(move || fetch(port, "/slow.php"));
    serve.wait_for("Accepted", 1);
    std::thread::sleep(Duration::from_millis(300));
    let killed = Command::new("kill").args(["-TERM", &serve.child.id().to_string()]).status().expect("kill");
    assert!(killed.success());
    let (head, body, _) = slow.join().expect("the slow request");
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n") && body == "slow", "{head}{body}");
    let started = Instant::now();
    let status = loop {
        if let Some(status) = serve.child.try_wait().expect("wait") {
            break status;
        }
        assert!(started.elapsed() < Duration::from_secs(20), "the server did not stop: {:?}", serve.log());
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(status.code(), Some(0), "{:?}", serve.log());
    serve.wait_for("p8 stopped", 1);
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err(), "nothing listens after it stopped");
}

/// Folders `a`, `b` and `c` of a project, each with `data.txt` naming it;
/// `a/move.php` reads its folder's file, moves to `b` and reads again
/// (after a pause, so another request runs meanwhile); `*/read.php` read
/// their own folder's file. Each prints its working directory's name.
fn folders(name: &str) -> PathBuf {
    let read = "<?php echo basename(getcwd()), ':', file_get_contents('data.txt');";
    let dir = site(name, &[]);
    for sub in ["a", "b", "c"] {
        std::fs::create_dir(dir.join(sub)).expect("a folder");
        std::fs::write(dir.join(sub).join("data.txt"), sub.to_uppercase()).expect("data.txt");
        std::fs::write(dir.join(sub).join("read.php"), read).expect("read.php");
    }
    let mv = "<?php echo basename(getcwd()), ':', file_get_contents('data.txt'), ' '; chdir('../b'); usleep(800000); echo basename(getcwd()), ':', file_get_contents('data.txt');";
    std::fs::write(dir.join("a/move.php"), mv).expect("move.php");
    dir
}

/// Each request starts in its script's folder (as PHP's servers start it),
/// and a chdir() is its own: with two workers, a request running at the
/// same time on the other worker, and the next request on the same worker,
/// each start in their own script's folder.
#[test]
fn each_request_starts_in_its_scripts_folder_and_its_chdir_is_its_own() {
    let dir = folders("cwd_two");
    let serve = Serve::start_with(&dir, &["--workers", "2"]);
    let port = serve.port;
    let moving = std::thread::spawn(move || fetch(port, "/a/move.php"));
    std::thread::sleep(Duration::from_millis(300));
    // On the other worker while the first has moved to b.
    assert_eq!(fetch(port, "/c/read.php").1, "c:C");
    assert_eq!(moving.join().expect("the moving request").1, "a:A b:B");
    // Next requests, whichever worker takes them (the one that moved too).
    for _ in 0..3 {
        assert_eq!(fetch(port, "/a/read.php").1, "a:A");
    }
}

/// The same with one worker: the request after a chdir() starts in its own
/// script's folder, not where the last one moved.
#[test]
fn a_chdir_is_not_left_to_the_next_request_on_the_worker() {
    let dir = folders("cwd_one");
    let serve = Serve::start_with(&dir, &["--workers", "1"]);
    assert_eq!(fetch(serve.port, "/a/move.php").1, "a:A b:B");
    assert_eq!(fetch(serve.port, "/a/read.php").1, "a:A");
    assert_eq!(fetch(serve.port, "/c/read.php").1, "c:C");
}

/// `p8 -S` with a router: the router runs in the directory the server
/// started in (php_cli_server.c does not chdir for it), and a script's
/// request in its own folder; no request moves the process's directory, so
/// the router still sees the start directory after a script moved its own.
#[test]
fn the_cli_server_moves_no_process_directory() {
    let dir = folders("cwd_cli");
    let started = dir.join("started-here");
    std::fs::create_dir(&started).expect("the start folder");
    let router = dir.join("router.php");
    std::fs::write(&router, "<?php if ($_SERVER['REQUEST_URI'] === '/cwd') { echo getcwd(); return true; } return false;").expect("router.php");
    let mut command = Command::new(env!("CARGO_BIN_EXE_p8"));
    command.current_dir(&started).args(["-S", "127.0.0.1:0", "-t", dir.to_str().expect("utf-8"), router.to_str().expect("utf-8")]);
    let serve = Serve::spawn(command);
    assert_eq!(fetch(serve.port, "/cwd").1, started.display().to_string());
    assert_eq!(fetch(serve.port, "/a/move.php").1, "a:A b:B");
    assert_eq!(fetch(serve.port, "/cwd").1, started.display().to_string());
    assert_eq!(fetch(serve.port, "/c/read.php").1, "c:C");
}

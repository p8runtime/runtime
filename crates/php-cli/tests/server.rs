//! `p8 -S`: PHP's built-in web server. Each case sends a raw request and
//! compares the whole raw response (status line, headers, body) with what
//! `php -S` of php:8.4-cli answered to the same bytes (tests/server/expected,
//! the dates, temporary file names, document root and PHP version replaced
//! by %DATE%, %TMP%, %ROOT% and %VERSION%). p8's target is PHP 8.6: what
//! 8.6's `php -S` answers differently (it no longer echoes Host, session
//! cookies are HttpOnly and SameSite=Lax by default, error pages have a
//! viewport tag) is re-recorded when p8 does it (#279); argv is not made
//! from the query string since register_argc_argv is off by default (8.5).
//!
//! To record the answers with real PHP, serve the same files from a
//! container and run the ignored test:
//!
//!     tar -cf - -C crates/php-cli/tests/server doc router.php | docker run -i --rm -p 127.0.0.1:18080:8080 php:8.4-cli \
//!         bash -c "mkdir /t && cd /t && tar xf - && php -S 0.0.0.0:8080 -t doc"
//!     (the same with `php -S 0.0.0.0:8080 -t doc router.php` on port 18081)
//!     ORACLE_PORT=18080 ORACLE_ROUTER_PORT=18081 cargo test -p php-cli --test server record_oracle -- --ignored

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn server_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/server")
}

/// A running `p8 -S` and what it logged.
struct Server {
    child: Child,
    port: u16,
    log: Arc<Mutex<Vec<String>>>,
}

impl Server {
    fn start(args: &[&str]) -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_p8")).args(["-S", "127.0.0.1:0"]).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().expect("start p8 -S");
        let stderr = child.stderr.take().expect("stderr");
        let log = Arc::new(Mutex::new(Vec::new()));
        let (tx, rx) = std::sync::mpsc::channel();
        let shared = Arc::clone(&log);
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if let Some(rest) = line.split("Development Server (http://127.0.0.1:").nth(1) {
                    let _ = tx.send(rest.split(')').next().unwrap_or("0").parse::<u16>().unwrap_or(0));
                }
                shared.lock().expect("log").push(line);
            }
        });
        let port = rx.recv_timeout(Duration::from_secs(20)).expect("the server's started line");
        Server { child, port, log }
    }

    fn docroot(extra: &[&str]) -> Server {
        let doc = server_dir().join("doc");
        let mut args = vec!["-t", doc.to_str().expect("path")];
        args.extend_from_slice(extra);
        Server::start(&args)
    }

    /// Send raw bytes and read the response to the end (the server closes).
    fn exchange(&self, request: &[u8]) -> Vec<u8> {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(20))).expect("timeout");
        stream.write_all(request).expect("send");
        let mut out = Vec::new();
        let _ = stream.read_to_end(&mut out);
        out
    }

    /// The log lines so far, once one satisfies `done`.
    fn log_until(&self, done: impl Fn(&[String]) -> bool) -> Vec<String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let lines = self.log.lock().expect("log").clone();
            if done(&lines) || Instant::now() > deadline {
                return lines;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A request as written in the cases (LF line ends, `{LEN}` for the body's length).
fn raw(text: &str) -> Vec<u8> {
    let crlf = text.replace('\n', "\r\n");
    let body_at = crlf.find("\r\n\r\n").map_or(crlf.len(), |i| i + 4);
    let len = crlf.len() - body_at;
    crlf.replace("{LEN}", &len.to_string()).into_bytes()
}

/// A response with what varies replaced.
/// `text` with each "PHP/<version>" (and its JSON form "PHP\/<version>")
/// as "PHP/%VERSION%": X-Powered-By, SERVER_SOFTWARE.
fn php_version_token(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    loop {
        let next = [rest.find("PHP/").map(|i| (i, 4)), rest.find("PHP\\/").map(|i| (i, 5))].into_iter().flatten().min();
        let Some((i, n)) = next else { break };
        out.push_str(&rest[..i + n]);
        let tail = &rest[i + n..];
        let len = tail.find(|c: char| !(c.is_ascii_alphanumeric() || c == '.')).unwrap_or(tail.len());
        if tail[..len].starts_with(|c: char| c.is_ascii_digit()) {
            out.push_str("%VERSION%");
        } else {
            out.push_str(&tail[..len]);
        }
        rest = &tail[len..];
    }
    out.push_str(rest);
    out
}

fn normalize(response: &[u8], root: &str) -> String {
    let text = String::from_utf8_lossy(response).replace(root, "%ROOT%");
    let mut out = String::new();
    for (i, line) in text.split_inclusive('\n').enumerate() {
        let _ = i;
        if line.starts_with("Date: ") {
            out.push_str("Date: %DATE%\r\n");
        } else {
            out.push_str(line);
        }
    }
    // php + 13 base-32 characters + 6: an uploaded file's name.
    let bytes = out.clone().into_bytes();
    let mut result = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"/php") && bytes.len() >= i + 4 + 19 && bytes[i + 4..i + 23].iter().all(u8::is_ascii_alphanumeric) {
            result.extend_from_slice(b"/%TMP%");
            i += 23;
        } else if bytes[i..].starts_with(b"Max-Age=") {
            // Seconds left: it depends on the time.
            result.extend_from_slice(b"Max-Age=N");
            i += 8;
            while bytes.get(i).is_some_and(u8::is_ascii_digit) {
                i += 1;
            }
        } else {
            result.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&result).into_owned()
}

/// (name, uses the router, request)
const CASES: &[(&str, bool, &str)] = &[
    ("static_text", false, "GET /hello.txt HTTP/1.1\nHost: localhost\n\n"),
    ("static_html_http10", false, "GET /page.html HTTP/1.0\nHost: localhost\n\n"),
    ("static_head", false, "HEAD /hello.txt HTTP/1.1\nHost: localhost\n\n"),
    ("not_found", false, "GET /sub/none.txt?a=<b>&c=1 HTTP/1.1\nHost: localhost\n\n"),
    ("method_not_allowed", false, "DELETE /hello.txt HTTP/1.1\nHost: localhost\n\n"),
    ("not_implemented", false, "FOO /hello.txt HTTP/1.1\nHost: localhost\n\n"),
    ("index_fallback", false, "GET /nope.zip HTTP/1.1\nHost: localhost\n\n"),
    ("path_info", false, "GET /info.php/extra/p?a=1&b[]=2 HTTP/1.1\nHost: localhost\nX-Custom-Header: some value\naccept: */*\n\n"),
    ("dot_segments", false, "GET /a/../sub/./../%68ello.txt HTTP/1.1\nHost: localhost\n\n"),
    (
        "post_urlencoded",
        false,
        "POST /form.php?x=1&y[]=2&z.w=3&a%20b=4 HTTP/1.1\nHost: localhost\nCookie: c1=v1; c2=a%20b; x=cookie\nContent-Type: application/x-www-form-urlencoded\nContent-Length: {LEN}\n\np=1&q[a]=2&q[b][]=3&x=post&%5Bbad=5&sp+ace=6",
    ),
    ("post_chunked", false, "POST /form.php HTTP/1.1\nHost: localhost\nContent-Type: application/x-www-form-urlencoded\nTransfer-Encoding: chunked\n\n5\na=1&b\n5\n=2&c=\n1\n3\n0\n\n"),
    ("put_body", false, "PUT /form.php HTTP/1.1\nHost: localhost\nContent-Type: text/plain\nContent-Length: {LEN}\n\nraw body here"),
    ("json_body", false, "POST /form.php HTTP/1.1\nHost: localhost\nContent-Type: application/json\nContent-Length: {LEN}\n\n{\"a\":1}"),
    (
        "multipart",
        false,
        "POST /form.php HTTP/1.1\nHost: localhost\nContent-Type: multipart/form-data; boundary=XyZ\nContent-Length: {LEN}\n\n--XyZ\nContent-Disposition: form-data; name=\"a\"\n\n1\n--XyZ\nContent-Disposition: form-data; name=\"b[]\"\n\n2\n--XyZ\nContent-Disposition: form-data; name=\"b[]\"\n\n3\n--XyZ\nContent-Disposition: form-data; name=\"f\"; filename=\"hello.txt\"\nContent-Type: text/plain\n\nhello\nupload\n--XyZ\nContent-Disposition: form-data; name=\"g[]\"; filename=\"C:\\dir\\page.html\"\nContent-Type: text/x-test; charset=x\n\n<p>page</p>\n--XyZ\nContent-Disposition: form-data; name=\"e\"; filename=\"\"\nContent-Type: application/octet-stream\n\n\n--XyZ\nContent-Disposition: form-data; name=\"h[x][y]\"; filename=\"a/b.txt\"\n\nnested\n--XyZ--\n",
    ),
    (
        "multipart_limits",
        false,
        "POST /form.php HTTP/1.1\nHost: localhost\nContent-Type: multipart/form-data; boundary=XyZ\nContent-Length: {LEN}\n\n--XyZ\nContent-Disposition: form-data; name=\"MAX_FILE_SIZE\"\n\n5\n--XyZ\nContent-Disposition: form-data; name=\"big\"; filename=\"big.txt\"\n\n0123456789\n--XyZ\nContent-Disposition: form-data; name=\"ok\"; filename=\"ok.txt\"\n\nabc\n--XyZ\nContent-Disposition: form-data; name=\"partial\"; filename=\"p.txt\"\n\nno end boundary",
    ),
    ("headers", false, "GET /headers.php HTTP/1.1\nHost: localhost\n\n"),
    ("redirect", false, "POST /redirect.php HTTP/1.1\nHost: localhost\nContent-Length: 0\n\n"),
    ("session_new", false, "GET /session.php?id=p8server1 HTTP/1.1\nHost: localhost\n\n"),
    ("session_cookie", false, "GET /session.php HTTP/1.1\nHost: localhost\nCookie: PHPSESSID=p8server2\n\n"),
    ("session_params", false, "GET /session.php?id=p8server3&params=1&setcookie=1&limiter= HTTP/1.1\nHost: localhost\n\n"),
    ("session_regenerate", false, "GET /session.php?id=p8server4&regenerate=1 HTTP/1.1\nHost: localhost\nCookie: PHPSESSID=p8server4\n\n"),
    ("errors_warn", false, "GET /errors.php HTTP/1.1\nHost: localhost\n\n"),
    ("errors_fatal", false, "GET /errors.php?what=fatal HTTP/1.1\nHost: localhost\n\n"),
    ("errors_exit", false, "GET /errors.php?what=exit HTTP/1.1\nHost: localhost\n\n"),
    ("errors_late", false, "GET /errors.php?what=late HTTP/1.1\nHost: localhost\n\n"),
    ("basic_auth", false, "GET /info.php HTTP/1.1\nHost: localhost\nAuthorization: Basic dXNlcjpwYXNz\n\n"),
    ("router_routed", true, "GET /anything?x=1 HTTP/1.1\nHost: localhost\n\n"),
    ("router_static", true, "GET /hello.txt HTTP/1.1\nHost: localhost\n\n"),
    ("router_declined_to_script", true, "GET /script HTTP/1.1\nHost: localhost\n\n"),
    ("router_php_file", true, "GET /script.php/more HTTP/1.1\nHost: localhost\n\n"),
];

fn root() -> String {
    std::fs::canonicalize(server_dir().join("doc")).expect("doc root").to_string_lossy().into_owned()
}

#[test]
fn answers_as_php_does() {
    let plain = Server::docroot(&[]);
    let router = server_dir().join("router.php");
    let routed = Server::docroot(&[router.to_str().expect("path")]);
    let root = root();
    let mut failures = Vec::new();
    for (name, uses_router, request) in CASES {
        let expected_path = server_dir().join("expected").join(format!("{name}.res"));
        let expected = std::fs::read_to_string(&expected_path).unwrap_or_else(|_| panic!("{} is missing: record it with real PHP (see the top of this file)", expected_path.display()));
        let server = if *uses_router { &routed } else { &plain };
        let response = server.exchange(&raw(request));
        // p8 adds its own product token after PHP's (p8::powered_by); the
        // rest of the response is PHP's.
        let text = String::from_utf8_lossy(&response);
        if text.contains("X-Powered-By: ") && !text.contains(&format!("X-Powered-By: {}", p8::powered_by())) {
            failures.push(format!("--- {name}
X-Powered-By is not {}", p8::powered_by()));
        }
        // Also as json_encode() shows a headers_list() (`\/`).
        let ours = format!(" p8/{}", p8::P8_VERSION);
        let ours_json = format!(" p8\\/{}", p8::P8_VERSION);
        let got = normalize(text.replace(&ours, "").replace(&ours_json, "").as_bytes(), &root);
        // The PHP version is the build's: what p8 reports (PHP_VERSION)
        // whatever release recorded the answers.
        let (got, expected) = (php_version_token(&got), php_version_token(&expected));
        if got != expected {
            failures.push(format!("--- {name}\nexpected:\n{expected}\ngot:\n{got}"));
        }
    }
    assert!(failures.is_empty(), "{} of {} cases differ from PHP:\n{}", failures.len(), CASES.len(), failures.join("\n"));
}

/// Records tests/server/expected/*.res from real PHP (see the top of this file).
#[test]
#[ignore]
fn record_oracle() {
    let port = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set")).parse::<u16>().expect("port");
    let (plain, routed) = (port("ORACLE_PORT"), port("ORACLE_ROUTER_PORT"));
    std::fs::create_dir_all(server_dir().join("expected")).expect("expected dir");
    for (name, uses_router, request) in CASES {
        let mut stream = TcpStream::connect(("127.0.0.1", if *uses_router { routed } else { plain })).expect("connect to the oracle");
        stream.write_all(&raw(request)).expect("send");
        let mut out = Vec::new();
        let _ = stream.read_to_end(&mut out);
        std::fs::write(server_dir().join("expected").join(format!("{name}.res")), normalize(&out, "/t/doc")).expect("write");
    }
}

#[test]
fn logs_each_request_the_way_php_does() {
    let server = Server::docroot(&[]);
    server.exchange(&raw("GET /hello.txt HTTP/1.1\nHost: localhost\n\n"));
    server.exchange(&raw("GET /sub/none.txt HTTP/1.1\nHost: localhost\n\n"));
    server.exchange(&raw("GET /errors.php?what=fatal HTTP/1.1\nHost: localhost\n\n"));
    let lines = server.log_until(|l| l.iter().filter(|x| x.ends_with(" Closing")).count() >= 3);
    // "[Thu Oct 29 06:47:19 2026] 127.0.0.1:PORT Accepted"
    let strip = |l: &String| l.split_once("] ").map(|(_, rest)| rest.to_string()).unwrap_or_default();
    // A message with newlines (a fatal error) continues on lines of its own.
    let mut rest: Vec<String> = Vec::new();
    for l in &lines {
        match rest.last_mut() {
            Some(last) if !l.starts_with('[') => {
                last.push('\n');
                last.push_str(l);
            }
            _ => rest.push(strip(l)),
        }
    }
    let without_port = |l: &str| -> String { l.split_once(' ').map(|(_, r)| r.to_string()).unwrap_or_default() };
    let shown: Vec<String> = rest.iter().skip(1).map(|l| without_port(l)).collect();
    let root = root();
    assert_eq!(&shown[0..3], ["Accepted", "[200]: GET /hello.txt", "Closing"]);
    assert_eq!(&shown[3..6], ["Accepted", "[404]: GET /sub/none.txt - No such file or directory", "Closing"]);
    assert_eq!(shown[6], "Accepted");
    let fatal = shown[7].replace(&root, "%ROOT%");
    assert_eq!(fatal, "[200]: GET /errors.php?what=fatal - Uncaught Exception: boom <i>fatal</i> in %ROOT%/errors.php:9\nStack trace:\n#0 {main}\n  thrown in %ROOT%/errors.php on line 9");
    assert_eq!(shown[8], "Closing");
    assert!(lines[0].contains("Development Server (http://127.0.0.1:"), "{}", lines[0]);
    // The timestamp is asctime(): "[Thu Oct 29 06:47:19 2026]".
    let stamp = &lines[1][1..lines[1].find(']').expect("]")];
    assert_eq!(stamp.split_whitespace().count(), 5, "{stamp}");
}

#[test]
fn a_silent_connection_blocks_nobody() {
    let server = Server::docroot(&[]);
    let _silent = TcpStream::connect(("127.0.0.1", server.port)).expect("connect");
    let response = server.exchange(&raw("GET /hello.txt HTTP/1.1\nHost: localhost\n\n"));
    assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 200 OK\r\n"));
    drop(_silent);
    let lines = server.log_until(|l| l.iter().any(|x| x.contains("Closed without sending a request")));
    assert!(lines.iter().any(|l| l.contains("Closed without sending a request; it was probably just an unused speculative preconnection")), "{lines:?}");
}

#[test]
fn a_malformed_request_is_logged_and_closed() {
    let server = Server::docroot(&[]);
    let response = server.exchange(b"get / HTTP/1.1\r\n\r\n");
    assert!(response.is_empty(), "{response:?}");
    let lines = server.log_until(|l| l.iter().any(|x| x.contains("Invalid request")));
    assert!(lines.iter().any(|l| l.ends_with("Invalid request (Malformed HTTP request)")), "{lines:?}");
    let response = server.exchange(b"\x16\x03\x01\x02\x00\x01\x00\x01\xfc\x03\x03");
    assert!(response.is_empty());
    let lines = server.log_until(|l| l.iter().any(|x| x.contains("Unsupported SSL request")));
    assert!(lines.iter().any(|l| l.ends_with("Invalid request (Unsupported SSL request)")), "{lines:?}");
}

#[test]
fn max_execution_time_ends_a_request_and_the_next_one_runs() {
    let scratch = scratch("time");
    std::fs::write(scratch.join("loop.php"), "<?php\necho \"start\\n\";\nwhile (true) {}\n").expect("write");
    std::fs::write(scratch.join("ok.php"), "<?php echo \"fine\";").expect("write");
    let server = Server::start(&["-d", "max_execution_time=1", "-t", scratch.to_str().expect("path")]);
    let started = Instant::now();
    let response = String::from_utf8_lossy(&server.exchange(&raw("GET /loop.php HTTP/1.1\nHost: localhost\n\n"))).into_owned();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(response.contains("start\n"), "{response}");
    assert!(response.contains("Maximum execution time of 1 second exceeded"), "{response}");
    // A later request is not cut short by the first one's timer.
    std::thread::sleep(Duration::from_millis(1500));
    assert!(String::from_utf8_lossy(&server.exchange(&raw("GET /ok.php HTTP/1.1\nHost: localhost\n\n"))).ends_with("fine"));
    let lines = server.log_until(|l| l.iter().any(|x| x.contains("[200]: GET /loop.php")));
    assert!(lines.iter().any(|l| l.contains("[200]: GET /loop.php - Maximum execution time of 1 second exceeded in ")), "{lines:?}");
}

/// A directory of our own to write scripts in.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("server-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

#[test]
fn a_changed_script_is_compiled_again() {
    let scratch = scratch("cache");
    let script = scratch.join("a.php");
    std::fs::write(&script, "<?php echo 'one';").expect("write");
    let server = Server::start(&["-t", scratch.to_str().expect("path")]);
    let get = || String::from_utf8_lossy(&server.exchange(&raw("GET /a.php HTTP/1.1\nHost: localhost\n\n"))).into_owned();
    assert!(get().ends_with("one"));
    assert!(get().ends_with("one"));
    // Same length, new content, a moment later.
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(&script, "<?php echo 'two';").expect("write");
    assert!(get().ends_with("two"));
    std::fs::write(&script, "<?php echo 'three, longer';").expect("write");
    assert!(get().ends_with("three, longer"));
    // A new file shows up, a removed one is gone.
    std::fs::write(scratch.join("b.php"), "<?php echo getcwd() === __DIR__ ? 'cwd' : 'elsewhere';").expect("write");
    assert!(String::from_utf8_lossy(&server.exchange(&raw("GET /b.php HTTP/1.1\nHost: localhost\n\n"))).ends_with("cwd"));
}

#[test]
fn refuses_what_php_refuses() {
    let run = |args: &[&str]| Command::new(env!("CARGO_BIN_EXE_p8")).args(args).output().expect("run");
    let out = run(&["-S", "nonsense"]);
    assert_eq!((out.status.code(), String::from_utf8_lossy(&out.stderr).as_ref()), (Some(1), "Invalid address: nonsense\n"));
    let out = run(&["-S", "127.0.0.1:99999"]);
    assert_eq!(String::from_utf8_lossy(&out.stderr), "Invalid address: 127.0.0.1:99999\n");
    let out = run(&["-S", "127.0.0.1:0", "-t", "/no/such/dir"]);
    assert_eq!((out.status.code(), String::from_utf8_lossy(&out.stderr).as_ref()), (Some(1), "Directory /no/such/dir does not exist.\n"));
    let file = server_dir().join("router.php");
    let out = run(&["-S", "127.0.0.1:0", "-t", file.to_str().expect("path")]);
    assert_eq!(String::from_utf8_lossy(&out.stderr), format!("{} is not a directory.\n", file.display()));
    // The port is taken.
    let server = Server::docroot(&[]);
    let address = format!("127.0.0.1:{}", server.port);
    let out = run(&["-S", &address]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.trim_end().ends_with(&format!("Failed to listen on {address} (reason: Address already in use)")), "{stderr}");
}

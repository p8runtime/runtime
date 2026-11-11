//! The curl extension over HTTP: each tests/curl/NAME.php must print what PHP
//! 8.4 (php:8.4-cli, libcurl 8.14) printed for it against the server below
//! (NAME.out, the script's file name replaced by %s). The scripts take the
//! server's URL from CURL_BASE and never print it.
//!
//! To record NAME.out with real PHP, serve on a fixed port and point the
//! container at the host:
//!
//!     cargo test -p php-cli --test curl serve_for_oracle -- --ignored --nocapture
//!     docker run --rm -e CURL_BASE=http://host.docker.internal:18080 ...

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

struct Request {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

struct Response {
    status: u16,
    reason: &'static str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    chunked: bool,
}

fn respond(status: u16, reason: &'static str, headers: &[(&str, &str)], body: impl Into<Vec<u8>>) -> Response {
    Response { status, reason, headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), body: body.into(), chunked: false }
}

/// A gzip stream of "hello gzip\n" (stored block, so no compressor is needed).
fn gzip_hello() -> Vec<u8> {
    let data = b"hello gzip\n";
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 3];
    out.extend_from_slice(&[1, data.len() as u8, 0, !(data.len() as u8), 0xff]);
    out.extend_from_slice(data);
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    out.extend_from_slice(&(!crc).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

fn route(req: &Request) -> Response {
    let (path, _query) = req.target.split_once('?').unwrap_or((&req.target, ""));
    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match segs.as_slice() {
        ["echo"] | ["cookie-show"] => {
            let mut body = format!("{} {}\n", req.method, req.target);
            let boundary = req.header("content-type").and_then(|c| c.split("boundary=").nth(1)).map(str::to_string);
            let mut headers: Vec<(String, String)> = req.headers.iter().filter(|(k, _)| !k.eq_ignore_ascii_case("host")).map(|(k, v)| (k.to_ascii_lowercase(), v.clone())).collect();
            headers.sort();
            for (k, v) in headers {
                let v = match &boundary {
                    Some(b) => v.replace(b.as_str(), "BOUNDARY"),
                    None => v,
                };
                // The multipart length varies with the boundary only through this header.
                body.push_str(&format!("{k}: {v}\n"));
            }
            body.push('\n');
            let mut payload = String::from_utf8_lossy(&req.body).into_owned();
            if let Some(b) = &boundary {
                payload = payload.replace(b.as_str(), "BOUNDARY");
            }
            body.push_str(&payload);
            respond(200, "OK", &[("Content-Type", "text/plain")], body)
        }
        ["r", code, target] => {
            let (code, reason) = match *code {
                "301" => (301, "Moved Permanently"),
                "302" => (302, "Found"),
                "303" => (303, "See Other"),
                "307" => (307, "Temporary Redirect"),
                _ => (308, "Permanent Redirect"),
            };
            respond(code, reason, &[("Location", &format!("/{target}")), ("Content-Type", "text/plain")], "moved\n")
        }
        ["loop"] => respond(302, "Found", &[("Location", "/loop")], ""),
        ["slow"] => {
            std::thread::sleep(Duration::from_millis(1500));
            respond(200, "OK", &[("Content-Type", "text/plain")], "late\n")
        }
        ["gzip"] => {
            if req.header("accept-encoding").is_some_and(|e| e.contains("gzip")) {
                respond(200, "OK", &[("Content-Type", "text/plain"), ("Content-Encoding", "gzip")], gzip_hello())
            } else {
                respond(200, "OK", &[("Content-Type", "text/plain")], "plain\n")
            }
        }
        ["status", code] => match *code {
            "404" => respond(404, "Not Found", &[("Content-Type", "text/plain")], "status 404\n"),
            _ => respond(500, "Internal Server Error", &[("Content-Type", "text/plain")], "status 500\n"),
        },
        ["auth"] => match req.header("authorization") {
            Some(a) => respond(200, "OK", &[("Content-Type", "text/plain")], format!("authed: {a}\n")),
            None => respond(401, "Unauthorized", &[("WWW-Authenticate", "Basic realm=\"t\""), ("Content-Type", "text/plain")], "no\n"),
        },
        ["cookie-set"] => respond(200, "OK", &[("Set-Cookie", "sid=abc; Path=/"), ("Set-Cookie", "theme=dark; Path=/"), ("Content-Type", "text/plain")], "cookies set\n"),
        ["chunked"] => {
            let mut r = respond(200, "OK", &[("Content-Type", "text/plain")], "chunk one,chunk two,chunk three\n");
            r.chunked = true;
            r
        }
        ["headers"] => respond(200, "OK", &[("X-A", "1"), ("X-B", "two words"), ("Content-Type", "text/html; charset=utf-8")], "<p>hi</p>\n"),
        ["big"] => respond(200, "OK", &[("Content-Type", "application/octet-stream")], "0123456789abcdef".repeat(8192)),
        _ => respond(404, "Not Found", &[("Content-Type", "text/plain")], "unknown\n"),
    }
}

fn read_request(r: &mut BufReader<TcpStream>) -> Option<Request> {
    let mut line = String::new();
    if r.read_line(&mut line).ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let mut headers = Vec::new();
    loop {
        let mut l = String::new();
        r.read_line(&mut l).ok()?;
        let l = l.trim_end();
        if l.is_empty() {
            break;
        }
        let (k, v) = l.split_once(':')?;
        headers.push((k.to_string(), v.trim().to_string()));
    }
    let mut body = Vec::new();
    let len = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-length")).and_then(|(_, v)| v.parse::<usize>().ok()).unwrap_or(0);
    body.resize(len, 0);
    r.read_exact(&mut body).ok()?;
    Some(Request { method, target, headers, body })
}

fn serve_connection(stream: TcpStream) {
    let mut writer = stream.try_clone().expect("clone");
    let mut reader = BufReader::new(stream);
    while let Some(req) = read_request(&mut reader) {
        let resp = route(&req);
        let mut out = format!("HTTP/1.1 {} {}\r\n", resp.status, resp.reason).into_bytes();
        for (k, v) in &resp.headers {
            out.extend_from_slice(format!("{k}: {v}\r\n").as_bytes());
        }
        let head = req.method == "HEAD";
        if resp.chunked {
            out.extend_from_slice(b"Transfer-Encoding: chunked\r\n\r\n");
            if !head {
                for part in resp.body.split_inclusive(|b| *b == b',') {
                    out.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
                    out.extend_from_slice(part);
                    out.extend_from_slice(b"\r\n");
                }
                out.extend_from_slice(b"0\r\n\r\n");
            }
        } else {
            out.extend_from_slice(format!("Content-Length: {}\r\n\r\n", resp.body.len()).as_bytes());
            if !head {
                out.extend_from_slice(&resp.body);
            }
        }
        if writer.write_all(&out).is_err() {
            return;
        }
    }
}

fn serve(listener: TcpListener) {
    for stream in listener.incoming().flatten() {
        std::thread::spawn(move || serve_connection(stream));
    }
}

/// Serves for recording expected output with real PHP (never ends).
#[test]
#[ignore = "serves forever on 0.0.0.0:18080 for recording expected output"]
fn serve_for_oracle() {
    serve(TcpListener::bind("0.0.0.0:18080").expect("bind"));
}

#[test]
fn curl_matches_php_8_4() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    std::thread::spawn(move || serve(listener));
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/curl");
    let mut scripts: Vec<_> = std::fs::read_dir(&dir)
        .expect("tests/curl")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "php"))
        .collect();
    scripts.sort();
    assert!(!scripts.is_empty(), "no scripts in {}", dir.display());
    let mut failures = Vec::new();
    for script in &scripts {
        let want = std::fs::read_to_string(script.with_extension("out")).expect("expected output");
        let out = Command::new(env!("CARGO_BIN_EXE_p8")).arg(script).env("CURL_BASE", format!("http://127.0.0.1:{port}")).output().expect("run p8");
        let shown = std::fs::canonicalize(script).expect("canonical path");
        let got = String::from_utf8_lossy(&out.stdout).replace(&*shown.to_string_lossy(), "%s");
        if got != want {
            let line = got.lines().zip(want.lines()).position(|(g, w)| g != w).unwrap_or(got.lines().count().min(want.lines().count()));
            failures.push(format!("{}: first difference at line {}\n  got:  {:?}\n  want: {:?}", script.display(), line + 1, got.lines().nth(line), want.lines().nth(line)));
        }
    }
    assert!(failures.is_empty(), "{} of {} scripts differ:\n{}", failures.len(), scripts.len(), failures.join("\n"));
}

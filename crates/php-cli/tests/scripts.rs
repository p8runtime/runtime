//! p8 end to end through the p8 host: each tests/scripts/NAME.php must print exactly
//! what real PHP printed for it (NAME.out, recorded reading the script from
//! stdin, its file name replaced by %s). Recorded with php:8.4-cli; p8's
//! target is PHP 8.6 since 4 Nov 2026, and a script is re-recorded with 8.6
//! when p8 does what 8.6 does for it (#279).

use std::path::Path;
use std::process::Command;

/// The scripts write fixed temporary files (zlib.php: p8zlib.gz), so the
/// runs of them take turns (and the memory_limit runs with them).
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn scripts_match_php_8_4() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    check(&[]);
}

/// The same scripts, every one taken from a package of them all
/// (`p8 compile --cli`, #381): the output is the same, byte for byte.
#[test]
fn scripts_match_php_8_4_from_a_package() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scripts");
    let package = Path::new(env!("CARGO_TARGET_TMPDIR")).join("scripts.package");
    let out = Command::new(env!("CARGO_BIN_EXE_p8")).arg("compile").arg(&dir).arg("--cli").arg("--package").arg(&package).output().expect("run p8 compile");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    check(&["--package".as_ref(), package.as_os_str()]);
}

/// The same scripts through a native executable of them all (`p8 compile
/// --native`, #402): the functions with machine code run as it, every
/// other one (most of them), and whatever the machine code hands back, on
/// the interpreter; the output is the same, byte for byte. Needs the
/// runtime library beside p8 (`cargo build --release --locked -p p8-native`).
#[test]
fn scripts_match_php_8_4_as_a_native_executable() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scripts");
    let tmp = Path::new(env!("CARGO_TARGET_TMPDIR"));
    let (package, exe) = (tmp.join("scripts.native.package"), tmp.join("scripts.native"));
    let host = Path::new(env!("CARGO_BIN_EXE_p8")).with_file_name("libp8_native.a");
    assert!(host.is_file(), "{} is missing: build it with `cargo build --release --locked -p p8-native`", host.display());
    let out = Command::new(env!("CARGO_BIN_EXE_p8"))
        .arg("compile")
        .arg(&dir)
        .arg("--package")
        .arg(&package)
        .arg("--native")
        .arg(&exe)
        .arg("--native-host")
        .arg(&host)
        .output()
        .expect("run p8 compile --native");
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    // (Some of the corpus's functions are native: else this tests nothing.)
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(!said.contains("with 0 of"), "{said}");
    check_with(exe.as_os_str(), &[]);
}

/// memory_limit stops a recursion that never ends (#445): the call stack
/// (register file and frame list) is charged against it as it grows, and
/// going past it is PHP's fatal error, which no `catch` sees. Shutdown
/// functions run after it, as after any fatal error.
/// (Without the fatal error's stack trace: at 16M it is some 130,000
/// frames deep.)
#[test]
fn memory_limit_stops_a_recursion_that_never_ends() {
    let (got, status) = run_bounded("forever.php", "16M", &["-d", "fatal_error_backtraces=0"]);
    let (message, rest) = fatal(&got, 16 << 20);
    assert_eq!(rest, " in %s on line 2\n\nshutdown: ".to_string() + &message + "\n");
    assert_eq!(status, Some(255));
}

/// The same with its stack trace (fatal_error_backtraces, on by default):
/// every call, under the limit's smaller.
#[test]
fn memory_limit_fatal_error_shows_the_calls() {
    let (got, status) = run_bounded("forever.php", "512K", &[]);
    let (message, rest) = fatal(&got, 512 << 10);
    assert!(rest.starts_with(" in %s on line 2\nStack trace:\n#0 %s(2): forever("), "{}", head(rest));
    let end = format!("): forever(1)\n#{n} %s(4): forever(0)\n#{m} {{main}}\n\nshutdown: {message}\n", n = rest.matches('\n').count() - 6, m = rest.matches('\n').count() - 5);
    assert!(rest.ends_with(&end), "want the end {end:?}, got {:?}", &rest[rest.len().saturating_sub(300)..]);
    assert_eq!(status, Some(255));
}

/// A recursion that fits in the limit still runs (#444's million calls).
#[test]
fn memory_limit_lets_a_deep_recursion_that_fits_run() {
    let (got, status) = run_bounded("depth.php", "512M", &[]);
    assert_eq!(got, "1000000\n");
    assert_eq!(status, Some(0));
}

/// `got` starts with PHP's out-of-memory fatal error at `limit` bytes,
/// not with what the script's `catch` prints: the error's message and
/// what follows it.
fn fatal(got: &str, limit: usize) -> (String, &str) {
    let prefix = format!("\nFatal error: Allowed memory size of {limit} bytes exhausted (tried to allocate ");
    assert!(got.starts_with(&prefix), "{}", head(got));
    let after = &got[prefix.len()..];
    let tried = &after[..after.find(' ').expect("the size")];
    assert!(tried.parse::<u64>().is_ok_and(|n| n > 0), "{}", head(got));
    let rest = after[tried.len()..].strip_prefix(" bytes)").expect("the size's unit");
    (format!("Allowed memory size of {limit} bytes exhausted (tried to allocate {tried} bytes)"), rest)
}

/// tests/memory_limit/`name` run with `-d memory_limit=limit` and
/// `options`, killed if it has not ended in 5 seconds (a regression must
/// not take the machine's memory): its output (the script's path as %s)
/// and exit status.
fn run_bounded(name: &str, limit: &str, options: &[&str]) -> (String, Option<i32>) {
    use std::process::Stdio;
    use std::time::{Duration, Instant};
    // (One at a time, and not beside the corpus: without the limit each
    // run takes memory as fast as it can.)
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/memory_limit").join(name);
    let mut child = Command::new(env!("CARGO_BIN_EXE_p8"))
        .args(["-d", &format!("memory_limit={limit}")])
        .args(options)
        .arg(&script)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("run p8");
    // The output is read as it comes (a full pipe would stop the child).
    let mut stdout = child.stdout.take().expect("its stdout");
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        std::io::Read::read_to_end(&mut stdout, &mut out).map(|_| out)
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait for p8") {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill().expect("kill p8");
            child.wait().expect("reap p8");
            panic!("{name} with memory_limit={limit} was still running after 5 s: killed");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let out = reader.join().expect("the reader").expect("p8's output");
    let shown = std::fs::canonicalize(&script).expect("canonical path");
    (String::from_utf8_lossy(&out).replace(&*shown.to_string_lossy(), "%s"), status.code())
}

/// The start of `text`, for a failure's message.
fn head(text: &str) -> &str {
    &text[..text.floor_char_boundary(400)]
}

/// Every tests/scripts/NAME.php run with `options` prints NAME.out.
fn check(options: &[&std::ffi::OsStr]) {
    check_with(env!("CARGO_BIN_EXE_p8").as_ref(), options);
}

/// The same, run by `exe`.
fn check_with(exe: &std::ffi::OsStr, options: &[&std::ffi::OsStr]) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/scripts");
    let mut scripts: Vec<_> = std::fs::read_dir(&dir)
        .expect("tests/scripts")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "php"))
        .collect();
    scripts.sort();
    assert!(!scripts.is_empty(), "no scripts in {}", dir.display());
    let mut failures = Vec::new();
    for script in &scripts {
        // Bytes: a script may print text in another encoding (XMLWriter's ISO-8859-1).
        let want = std::fs::read(script.with_extension("out")).expect("expected output");
        let out = Command::new(exe).args(options).arg(script).output().expect("run p8");
        if String::from_utf8_lossy(&out.stderr).contains("p8: not using the package") {
            failures.push(format!("{}: the package was not used", script.display()));
        }
        let shown = std::fs::canonicalize(script).expect("canonical path");
        let got = replace(&out.stdout, shown.to_string_lossy().as_bytes(), b"%s");
        if got != want {
            let (got, want) = (String::from_utf8_lossy(&got), String::from_utf8_lossy(&want));
            let line = got.lines().zip(want.lines()).position(|(g, w)| g != w).unwrap_or(got.lines().count().min(want.lines().count()));
            failures.push(format!(
                "{}: first difference at line {}\n  got:  {:?}\n  want: {:?}",
                script.display(),
                line + 1,
                got.lines().nth(line),
                want.lines().nth(line)
            ));
        }
    }
    assert!(failures.is_empty(), "{} of {} scripts differ:\n{}", failures.len(), scripts.len(), failures.join("\n"));
}

/// `haystack` with every `from` replaced by `to`.
fn replace(haystack: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(haystack.len());
    let mut i = 0;
    while i < haystack.len() {
        if !from.is_empty() && haystack[i..].starts_with(from) {
            out.extend_from_slice(to);
            i += from.len();
        } else {
            out.push(haystack[i]);
            i += 1;
        }
    }
    out
}

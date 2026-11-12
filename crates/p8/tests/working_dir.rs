//! Each request has its own working directory (#459): a script's chdir()
//! moves only its own request's, so two requests running at once on two
//! threads of one process each resolve relative paths, includes and child
//! processes against their own folder.

use std::path::{Path, PathBuf};

#[derive(Default)]
struct Capture(Vec<u8>);

impl p8::Host for Capture {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
}

fn platform() -> p8::Platform {
    p8::Platform::new(&[p8::extensions::STANDARD])
}

/// A new empty folder for one test, by its real path (getcwd() and `pwd -P`
/// give real paths; the temporary directory may be under a link).
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("p8-working-dir-{name}-{}", std::process::id()));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("an old scratch folder removed");
    }
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    std::fs::canonicalize(&dir).expect("its real path")
}

/// A request's folder: `data.txt` and `inc.php` say which folder they are
/// in; only `b` has a file named `only-b`.
fn folder(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir(&dir).expect("a folder");
    std::fs::write(dir.join("data.txt"), name.to_uppercase()).expect("data.txt");
    std::fs::write(dir.join("inc.php"), format!("<?php echo 'inc-{name}', \"\\n\";")).expect("inc.php");
    if name == "b" {
        std::fs::write(dir.join("only-b"), "").expect("only-b");
    }
    dir
}

fn run(platform: &p8::Platform, source: &str, start: Option<&Path>) -> (i32, String) {
    run_with(platform, source, start, &[])
}

/// [`run`] with ini settings made for the request.
fn run_with(platform: &p8::Platform, source: &str, start: Option<&Path>, ini: &[(&str, &str)]) -> (i32, String) {
    let script = platform.compile(source.as_bytes(), "t.php").expect("compiles");
    let mut isolate = platform.new_isolate();
    let mut host = Capture::default();
    let status = {
        let mut context = isolate.new_context(&mut host);
        for (name, value) in ini {
            context.set_ini(name, value);
        }
        if let Some(dir) = start {
            context.set_working_dir(dir);
        }
        context.run(&script)
    };
    (status, String::from_utf8(host.0).expect("utf-8"))
}

/// A name that exists relative to the process's directory (this crate's,
/// where cargo runs the test) and not relative to the request's: a builtin
/// that still resolved against the process's would find it.
const PROCESS_ONLY: &str = "src";

#[test]
fn tempnam_never_uses_a_folder_of_the_process_directory() {
    let root = scratch("tempnam");
    assert!(Path::new(PROCESS_ONLY).is_dir() && !root.join(PROCESS_ONLY).exists());
    let source = format!("<?php $f = tempnam('{PROCESS_ONLY}', 'p8cwdtest'); echo $f, \"\\n\"; unlink($f);");
    let (status, out) = run(&platform(), &source, Some(&root));
    // A file left in the process's folder by the old resolution goes.
    for e in std::fs::read_dir(PROCESS_ONLY).expect("src/").flatten() {
        if e.file_name().to_string_lossy().starts_with("p8cwdtest") {
            let _ = std::fs::remove_file(e.path());
        }
    }
    let temp = std::fs::canonicalize(std::env::temp_dir()).expect("the temporary directory");
    assert_eq!(status, 0);
    assert!(out.contains("tempnam(): file created in the system's temporary directory"), "{out}");
    let made = out.lines().last().expect("the file's name");
    assert!(made.starts_with(&format!("{}/p8cwdtest", temp.display())), "{out}");
    std::fs::remove_dir_all(&root).expect("the scratch folder removed");
}

#[test]
fn opcache_finds_a_script_by_the_request_s_relative_name() {
    let root = scratch("opcache");
    std::fs::write(root.join("x.php"), "<?php return 1;").expect("x.php");
    let on = |name: &str| matches!(name, "opcache.enable" | "opcache.enable_cli").then(|| "1".to_string());
    let platform = platform().opcache(p8::OpcacheSettings::from_ini(true, &on)).keep_compiled();
    let source = format!(
        "<?php var_dump(include 'x.php'); var_dump(opcache_is_script_cached('{abs}'), opcache_is_script_cached('x.php'), opcache_invalidate('x.php', true), opcache_is_script_cached('{abs}'));",
        abs = root.join("x.php").display()
    );
    let ini = [("opcache.enable", "1"), ("opcache.enable_cli", "1")];
    assert_eq!(run_with(&platform, &source, Some(&root), &ini), (0, "int(1)\nbool(true)\nbool(true)\nbool(true)\nbool(false)\n".into()));
    std::fs::remove_dir_all(&root).expect("the scratch folder removed");
}

#[test]
fn a_unix_socket_is_bound_and_reached_in_the_request_s_directory() {
    let root = scratch("socket");
    // A short name under a directory too long for sun_path (104 bytes on
    // macOS, 108 on Linux) with the directory joined to it.
    let long = root.join("d".repeat(110usize.saturating_sub(root.as_os_str().len()).max(8)));
    std::fs::create_dir(&long).expect("a long folder");
    let platform = p8::Platform::new(&[p8::extensions::STANDARD, p8::extensions::SOCKETS]);
    let source = "<?php
        $s = socket_create(AF_UNIX, SOCK_STREAM, 0);
        var_dump(socket_bind($s, 's.sock'), file_exists('s.sock'), is_file('s.sock'));
        socket_getsockname($s, $name); var_dump($name);
        $c = socket_create(AF_UNIX, SOCK_STREAM, 0);
        var_dump(socket_connect($c, 's.sock'));
        socket_getpeername($c, $peer); var_dump($peer);
        socket_close($c); socket_close($s); unlink('s.sock');";
    let want = "bool(true)\nbool(true)\nbool(false)\nstring(6) \"s.sock\"\nbool(true)\nstring(6) \"s.sock\"\n";
    for dir in [&root, &long] {
        let (status, out) = run(&platform, source, Some(dir));
        let _ = std::fs::remove_file("s.sock");
        assert_eq!((status, out.as_str()), (0, want), "in {} ({} bytes)", dir.display(), dir.as_os_str().len());
        assert!(!dir.join("s.sock").exists());
    }
    std::fs::remove_dir_all(&root).expect("the scratch folder removed");
}

#[test]
fn mysqli_connects_to_a_relative_socket_in_the_request_s_directory() {
    let root = scratch("mysqli");
    // Not a server: it takes the connection and closes it, so a client that
    // reached it fails reading the greeting (2013), not finding it (2002).
    let listener = std::os::unix::net::UnixListener::bind(root.join("m.sock")).expect("a socket");
    listener.set_nonblocking(true).expect("non-blocking");
    let server = std::thread::spawn(move || accept_for_a_while(|| listener.accept().map(drop)));
    let platform = p8::Platform::new(&[p8::extensions::STANDARD, p8::extensions::MYSQLI]);
    let source = "<?php mysqli_report(MYSQLI_REPORT_OFF); $m =@mysqli_connect('localhost', 'u', 'p', '', 0, 'm.sock'); echo mysqli_connect_errno();";
    let (status, out) = run(&platform, source, Some(&root));
    server.join().expect("the socket's thread");
    assert_eq!(status, 0);
    assert_ne!(out, "2002", "the socket was looked for elsewhere");
    assert!(out.parse::<i64>().is_ok_and(|n| n > 0), "{out}");
    std::fs::remove_dir_all(&root).expect("the scratch folder removed");
}

#[test]
fn curl_reads_a_relative_ca_file_in_the_request_s_directory() {
    let root = scratch("curl");
    let system = ["/etc/ssl/cert.pem", "/etc/ssl/certs/ca-certificates.crt"].into_iter().find(|p| Path::new(p).is_file()).expect("a system CA bundle");
    std::fs::copy(system, root.join("ca.pem")).expect("ca.pem");
    // Not a TLS server: it takes connections and closes them. A CA file
    // that loads makes that a handshake failure; one not found fails
    // before any connection, with another error: so the file in the
    // request's folder and a missing one must fail differently.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let port = listener.local_addr().expect("its address").port();
    listener.set_nonblocking(true).expect("non-blocking");
    let server = std::thread::spawn(move || accept_for_a_while(|| listener.accept().map(drop)));
    let platform = p8::Platform::new(&[p8::extensions::STANDARD, p8::extensions::CURL]);
    let source = format!(
        "<?php foreach (['ca.pem', 'missing.pem'] as $ca) {{ $h = curl_init('https://127.0.0.1:{port}/'); curl_setopt($h, CURLOPT_CAINFO, $ca); curl_setopt($h, CURLOPT_TIMEOUT, 5); curl_setopt($h, CURLOPT_RETURNTRANSFER, true); curl_exec($h); echo curl_errno($h), ' ', curl_error($h), \"\\n\"; }}"
    );
    let (status, out) = run(&platform, &source, Some(&root));
    server.join().expect("the server's thread");
    assert_eq!(status, 0);
    let codes: Vec<&str> = out.lines().collect();
    assert_eq!(codes.len(), 2, "{out}");
    assert_ne!(codes[0], codes[1], "ca.pem in the request's folder was not found: {out}");
    std::fs::remove_dir_all(&root).expect("the scratch folder removed");
}

/// A stand-in server's accept loop: takes what connects (and drops it)
/// for 5 seconds, then stops, so a client that never connects ends the
/// test instead of hanging it.
fn accept_for_a_while(mut accept: impl FnMut() -> std::io::Result<()>) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < until {
        if accept().is_err() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

/// PHP that waits until the file `flag` exists: a handshake between the two
/// requests (each step waits for the other's file, never for a time).
fn wait_for(flag: &Path) -> String {
    format!("$n = 0; while (!file_exists('{}')) {{ if (++$n > 200000) {{ echo \"timed out\\n\"; exit(1); }} usleep(100); }}\n", flag.display())
}

#[test]
fn a_chdir_in_one_request_moves_no_other_request() {
    let root = scratch("two");
    let (a, b) = (folder(&root, "a"), folder(&root, "b"));
    let flags = root.join("flags");
    std::fs::create_dir(&flags).expect("flags");
    let process_dir = std::env::current_dir().expect("the test's directory");

    // B moves into its folder first; then A moves into its own while B
    // waits; then B reads everything relative while A waits for it.
    let first = format!(
        "<?php\n{wait_b}chdir('{a}');\ntouch('{flags}/a-in');\n{wait_done}echo getcwd(), \"\\n\", file_get_contents('data.txt'), \"\\n\";\nvar_dump(file_exists('only-b'));\n",
        wait_b = wait_for(&flags.join("b-in")),
        wait_done = wait_for(&flags.join("b-done")),
        a = a.display(),
        flags = flags.display(),
    );
    let second = format!(
        "<?php\nchdir('{b}');\ntouch('{flags}/b-in');\n{wait_a}echo getcwd(), \"\\n\";\necho file_get_contents('data.txt'), \"\\n\";\nvar_dump(file_exists('only-b'), is_file('data.txt'));\n$f = fopen('data.txt', 'r'); echo fread($f, 10), \"\\n\"; fclose($f);\ninclude 'inc.php';\ninclude './inc.php';\necho trim(shell_exec('pwd -P')), \"\\n\";\ntouch('{flags}/b-done');\n",
        wait_a = wait_for(&flags.join("a-in")),
        b = b.display(),
        flags = flags.display(),
    );

    let platform = std::sync::Arc::new(platform());
    let threads: Vec<_> = [first, second]
        .into_iter()
        .map(|source| {
            let platform = platform.clone();
            std::thread::spawn(move || run(&platform, &source, None))
        })
        .collect();
    let results: Vec<(i32, String)> = threads.into_iter().map(|t| t.join().expect("a request's thread")).collect();

    assert_eq!(results[0], (0, format!("{}\nA\nbool(false)\n", a.display())), "the request that moved to a");
    assert_eq!(
        results[1],
        (0, format!("{b}\nB\nbool(true)\nbool(true)\nB\ninc-b\ninc-b\n{b}\n", b = b.display())),
        "the request in b, after the other moved to a"
    );
    assert_eq!(std::env::current_dir().expect("the test's directory"), process_dir, "a script moved the process's directory");
    std::fs::remove_dir_all(&root).expect("the scratch folder removed");
}

#[test]
fn a_host_sets_where_a_request_starts() {
    let root = scratch("start");
    let b = folder(&root, "b");
    let process_dir = std::env::current_dir().expect("the test's directory");
    let source = "<?php echo getcwd(), \"\\n\", file_get_contents('data.txt'), \"\\n\"; include 'inc.php'; chdir('..'); echo getcwd(), \"\\n\";";

    assert_eq!(run(&platform(), source, Some(&b)), (0, format!("{}\nB\ninc-b\n{}\n", b.display(), root.display())));
    // A relative start is taken against where the request would have started.
    let rel = b.strip_prefix(&process_dir).map(Path::to_path_buf).ok();
    if let Some(rel) = rel {
        assert_eq!(run(&platform(), "<?php echo getcwd();", Some(&rel)).1, b.display().to_string());
    }
    // Without one, a request starts in the process's directory.
    assert_eq!(run(&platform(), "<?php echo getcwd();", None).1, process_dir.display().to_string());
    assert_eq!(std::env::current_dir().expect("the test's directory"), process_dir);

    let platform = platform();
    let script = platform.compile(b"<?php chdir('..');", "t.php").expect("compiles");
    let mut isolate = platform.new_isolate();
    let mut host = Capture::default();
    let mut context = isolate.new_context(&mut host);
    context.set_working_dir(&b);
    assert_eq!(context.working_dir(), b);
    assert_eq!(context.run(&script), 0);
    assert_eq!(context.working_dir(), root, "chdir() moves the request's directory");
    drop(context);
    std::fs::remove_dir_all(&root).expect("the scratch folder removed");
}

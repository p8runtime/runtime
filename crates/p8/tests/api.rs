//! The embedding API's promises: a context's output goes to its host, and
//! nothing a request leaves behind (globals, functions, constants, output
//! buffers) is seen by the next request on the same isolate.

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

/// Run `source` as one request on `isolate`; its exit status and output.
fn request(platform: &p8::Platform, isolate: &mut p8::Isolate, source: &str) -> (i32, String) {
    let mut host = Capture::default();
    let status = {
        let mut context = isolate.new_context(&mut host);
        match platform.compile(source.as_bytes(), "t.php") {
            Ok(script) => context.run(&script),
            Err(error) => context.fail(&error),
        }
    };
    (status, String::from_utf8(host.0).expect("utf-8"))
}

#[test]
fn output_goes_to_the_host() {
    let platform = platform();
    let mut isolate = platform.new_isolate();
    assert_eq!(request(&platform, &mut isolate, "<?php echo 1 + 2, \"\\n\";"), (0, "3\n".into()));
}

#[test]
fn requests_on_one_isolate_share_nothing() {
    let platform = platform();
    let mut isolate = platform.new_isolate();
    let first = "<?php $x = 'kept?'; define('C', 1); function f() { return 1; } ob_start(); echo 'buffered';";
    let (status, out) = request(&platform, &mut isolate, first);
    // An open output buffer is flushed when the request ends.
    assert_eq!((status, out.as_str()), (0, "buffered"));

    let second = "<?php var_dump(isset($x), defined('C'), function_exists('f'));";
    let (status, out) = request(&platform, &mut isolate, second);
    assert_eq!(status, 0);
    assert_eq!(out, "bool(false)\nbool(false)\nbool(false)\n");
}

#[test]
fn a_script_compiled_once_runs_in_many_requests() {
    let platform = platform();
    let mut isolate = platform.new_isolate();
    let script = platform.compile(b"<?php function f($n) { return $n * 2; } echo f(21);", "t.php").expect("compiles");
    for _ in 0..3 {
        let mut host = Capture::default();
        let status = isolate.new_context(&mut host).run(&script);
        assert_eq!((status, host.0.as_slice()), (0, b"42".as_slice()));
    }
}

#[test]
fn a_failed_request_leaves_the_isolate_usable() {
    let platform = platform();
    let mut isolate = platform.new_isolate();
    let (status, out) = request(&platform, &mut isolate, "<?php $a = [1, 2]; echo intdiv_missing(1);");
    assert_eq!(status, 255);
    assert!(out.contains("Call to undefined function intdiv_missing()"), "{out}");
    assert_eq!(request(&platform, &mut isolate, "<?php echo isset($a) ? 'leak' : 'clean';"), (0, "clean".into()));
}

#[test]
fn compile_errors_are_reported_as_php_does() {
    let platform = platform();
    let mut isolate = platform.new_isolate();
    let (status, out) = request(&platform, &mut isolate, "<?php echo 1 +;");
    assert_eq!(status, 255);
    assert!(out.starts_with("\nParse error: ") && out.ends_with(" in t.php on line 1\n"), "{out:?}");
}

#[test]
fn a_script_is_shared_by_isolates_on_many_threads() {
    fn shared<T: Send + Sync>() {}
    shared::<p8::Script>();
    shared::<p8::Platform>();

    let platform = std::sync::Arc::new(platform());
    let script = std::sync::Arc::new(
        platform.compile(b"<?php function f($n) { return $n < 2 ? $n : f($n - 1) + f($n - 2); } echo f(15);", "t.php").expect("compiles"),
    );
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let (platform, script) = (platform.clone(), script.clone());
            std::thread::spawn(move || {
                let mut isolate = platform.new_isolate();
                let mut host = Capture::default();
                let status = isolate.new_context(&mut host).run(&script);
                (status, host.0)
            })
        })
        .collect();
    for t in threads {
        assert_eq!(t.join().expect("thread"), (0, b"610".to_vec()));
    }
}

#[test]
fn what_a_request_resolves_is_not_cached_for_the_next() {
    // The same compiled script, two requests: each defines C itself, so
    // each must see its own value (a resolution cached in shared code would
    // hand the second request the first one's).
    let platform = platform();
    let mut isolate = platform.new_isolate();
    let script = platform.compile(b"<?php define('C', hrtime(true)); echo C === hrtime(true) ? '' : C;", "t.php").expect("compiles");
    let mut outputs = Vec::new();
    for _ in 0..2 {
        let mut host = Capture::default();
        assert_eq!(isolate.new_context(&mut host).run(&script), 0);
        outputs.push(host.0);
    }
    assert_ne!(outputs[0], outputs[1], "the second request saw the first request's constant");
}

/// Run `source` while another thread interrupts it after 50 ms.
fn interrupted(source: &str, interrupt: fn(&p8::InterruptHandle)) -> (i32, String, p8::Isolate) {
    let platform = platform();
    let mut isolate = platform.new_isolate();
    let handle = isolate.interrupt_handle();
    let script = platform.compile(source.as_bytes(), "t.php").expect("compiles");
    let watchdog = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        interrupt(&handle);
    });
    let mut host = Capture::default();
    let status = isolate.new_context(&mut host).run(&script);
    watchdog.join().expect("watchdog");
    (status, String::from_utf8(host.0).expect("utf-8"), isolate)
}

#[test]
fn a_watchdog_stops_a_loop_running_in_the_fast_path() {
    let (status, out, _) = interrupted("<?php\n$i = 0;\nwhile (true) { $i++; }", |h| h.timeout(1));
    assert_eq!(status, 255);
    assert_eq!(out, "\nFatal error: Maximum execution time of 1 second exceeded in t.php on line 3\n");
}

#[test]
fn a_watchdog_stops_loops_and_recursion_outside_the_fast_path() {
    let (status, out, _) = interrupted("<?php\n$s = '';\nwhile (true) { $s = 'a' . 'b'; }", |h| h.timeout(30));
    assert_eq!((status, out.as_str()), (255, "\nFatal error: Maximum execution time of 30 seconds exceeded in t.php on line 3\n"));

    let source = "<?php\nfunction f($n) { return $n < 1 ? 0 : f($n - 1) + f($n - 1); }\nf(60);";
    let (status, out, _) = interrupted(source, |h| h.terminate());
    assert_eq!(status, 255);
    assert!(out.starts_with("\nFatal error: Execution was terminated by the host in t.php on line 2"), "{out:?}");
}

#[test]
fn an_interrupted_isolate_runs_the_next_request() {
    let (status, _, mut isolate) = interrupted("<?php while (true) {}", |h| h.terminate());
    assert_eq!(status, 255);
    let platform = platform();
    assert_eq!(request(&platform, &mut isolate, "<?php for ($i = 0; $i < 3; $i++) echo $i;"), (0, "012".into()));
}

/// A broken heredoc fails to compile, in bounded time: compilation can't be
/// interrupted, so a lexer loop would stall the host for good.
#[test]
fn unterminated_heredoc_is_a_parse_error_not_a_hang() {
    for source in ["<?php\n\necho <<<END\n", "<?php\n$b = <<<EOT\n\\\nEOT;\n", "<?php\n$b = <<<'EOT'\nx\n"] {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let platform = platform();
            let _ = tx.send(platform.compile(source.as_bytes(), "t.php").err().map(|e| (e.kind, e.message)));
        });
        let result = rx.recv_timeout(std::time::Duration::from_secs(5)).expect("compile returned within 5 s");
        if source.contains("\\\n") {
            assert!(result.is_none(), "a backslash before the closing label is valid: {result:?}");
        } else {
            let (kind, message) = result.expect("a parse error");
            assert!(matches!(kind, p8::ErrorKind::Parse), "{kind:?}");
            assert_eq!(message, "syntax error, unexpected end of file");
        }
    }
}

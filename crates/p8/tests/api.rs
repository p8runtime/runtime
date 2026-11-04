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
fn literals_written_in_one_request_are_as_written_in_the_next() {
    // An isolate makes a script's literal strings and arrays once; every
    // request's writes go to copies.
    let platform = platform();
    let mut isolate = platform.new_isolate();
    let source = b"<?php
        class C { const L = ['a' => [1]]; public $p = ['x' => ['y' => 1]]; public static $s = ['n' => 0]; public $t = 'abc'; }
        function f() { $a = ['k' => ['d' => 'v'], 'l' => [1, 2]]; $a['k']['d'] .= '!'; $a['l'][] = 3; $s = 'literal'; $s[0] = 'L'; return $a['k']['d'] . count($a['l']) . $s; }
        $c = new C; $c->p['x']['y']++; $c->t[0] = 'X'; C::$s['n']++; $l = C::L; $l['a'][] = 2;
        $g = fn() => ['v' => 'w']; $x = $g(); $x['v'] = 'changed';
        echo f(), f(), ' ', $c->p['x']['y'], $c->t, (new C)->t, C::$s['n'], count($l['a']), count(C::L['a']), $g()['v'], $x['v'];";
    let script = platform.compile(source, "t.php").expect("compiles");
    for _ in 0..3 {
        let mut host = Capture::default();
        let status = isolate.new_context(&mut host).run(&script);
        assert_eq!((status, String::from_utf8_lossy(&host.0).into_owned()), (0, "v!3Literalv!3Literal 2Xbcabc121wchanged".to_string()));
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
fn interrupted(platform: &p8::Platform, source: &str, interrupt: fn(&p8::InterruptHandle)) -> (i32, String, p8::Isolate) {
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
    let (status, out, _) = interrupted(&platform(), "<?php\n$i = 0;\nwhile (true) { $i++; }", |h| h.timeout(1));
    assert_eq!(status, 255);
    assert_eq!(out, "\nFatal error: Maximum execution time of 1 second exceeded in t.php on line 3\n");
}

#[test]
fn a_watchdog_stops_loops_and_recursion_outside_the_fast_path() {
    let (status, out, _) = interrupted(&platform(), "<?php\n$s = '';\nwhile (true) { $s = 'a' . 'b'; }", |h| h.timeout(30));
    assert_eq!((status, out.as_str()), (255, "\nFatal error: Maximum execution time of 30 seconds exceeded in t.php on line 3\n"));

    let source = "<?php\nfunction f($n) { return $n < 1 ? 0 : f($n - 1) + f($n - 1); }\nf(60);";
    let (status, out, _) = interrupted(&platform(), source, |h| h.terminate());
    assert_eq!(status, 255);
    assert!(out.starts_with("\nFatal error: Execution was terminated by the host in t.php on line 2"), "{out:?}");
}

#[test]
fn an_interrupted_isolate_runs_the_next_request() {
    let platform = platform();
    let (status, _, mut isolate) = interrupted(&platform, "<?php while (true) {}", |h| h.terminate());
    assert_eq!(status, 255);
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

/// A platform that keeps compiled files: a file included again runs as it
/// did the first time (its `static` variables start over, as in PHP with
/// or without opcache), and a file whose source changed between requests
/// runs its new code.
#[test]
fn kept_compiled_files_run_as_new_and_follow_changes() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("keep_compiled");
    std::fs::create_dir_all(&dir).expect("test dir");
    let part = dir.join("part.php");
    let main = format!("<?php include {0:?}; include {0:?}; echo \"\\n\";", part.to_str().expect("utf-8 path"));
    let platform = platform().keep_compiled();
    let mut isolate = platform.new_isolate();
    std::fs::write(&part, "<?php static $n = 0; echo ++$n;").expect("write part");
    assert_eq!(request(&platform, &mut isolate, &main), (0, "11\n".into()));
    assert_eq!(request(&platform, &mut isolate, &main), (0, "11\n".into()));
    std::fs::write(&part, "<?php static $n = 10; echo ++$n;").expect("rewrite part");
    assert_eq!(request(&platform, &mut isolate, &main), (0, "1111\n".into()));
}

/// opcache.file_cache: a second process (here: a second Platform, with
/// its own symbols) compiles nothing and runs the same; a changed file is
/// compiled again, and a damaged cache entry is compiled over.
#[test]
fn a_file_cache_serves_later_platforms() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("file_cache");
    let cache = dir.join("cache");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("test dir");
    let part = dir.join("part.php");
    std::fs::write(&part, "<?php class P { const C = [1, 'two' => 2.5]; static function f(int $x): string { return __CLASS__ . $x . strtoupper('a'); } } echo P::f(1), P::C['two'], basename(__FILE__);").expect("write part");
    let main = format!("<?php include {:?}; echo ' ', (fn($y) => $y * 2)(21), \"\\n\";", part.to_str().expect("utf-8 path"));
    let run = || {
        // Another Platform interns other names first, so its symbols differ.
        let platform = p8::Platform::new(&[p8::extensions::STANDARD]).file_cache(&cache);
        let _ = platform.compile(b"<?php class Other {} function other() {}", "other.php");
        let before = platform.compiled_files();
        let mut isolate = platform.new_isolate();
        let out = request(&platform, &mut isolate, &main);
        (out, platform.compiled_files() - before)
    };
    let expected = ((0, "P1A2.5part.php 42\n".to_string()), 2);
    assert_eq!(run(), expected, "first run compiles both files");
    assert_eq!(run(), ((0, "P1A2.5part.php 42\n".to_string()), 0), "second run compiles nothing");
    // Without the cache, the same output.
    let platform = platform();
    let mut isolate = platform.new_isolate();
    assert_eq!(request(&platform, &mut isolate, &main), expected.0);
    // A changed file: compiled again, its entry written over (one per file).
    std::fs::write(&part, "<?php echo 'changed';").expect("rewrite part");
    assert_eq!(run(), ((0, "changed 42\n".to_string()), 1));
    assert_eq!(run(), ((0, "changed 42\n".to_string()), 0));
    // Damaged entries (cut short, one byte changed): compiled over.
    let entries: Vec<_> = std::fs::read_dir(&cache).expect("cache dir").map(|e| e.expect("entry").path()).collect();
    assert_eq!(entries.len(), 3, "other.php, t.php and part.php: {entries:?}");
    for (i, entry) in entries.iter().enumerate() {
        let mut bytes = std::fs::read(entry).expect("entry");
        if i % 2 == 0 {
            bytes.truncate(bytes.len() / 2);
        } else {
            let at = bytes.len() - 3;
            bytes[at] ^= 1;
        }
        std::fs::write(entry, bytes).expect("damage");
    }
    assert_eq!(run(), ((0, "changed 42\n".to_string()), 2));
    assert_eq!(run(), ((0, "changed 42\n".to_string()), 0));
}

#[test]
#[should_panic(expected = "Platform that compiled it")]
fn a_script_runs_only_where_its_names_mean_the_same() {
    // Names compile to the compiling Platform's symbols: another
    // Platform's isolate would read them as other names.
    let (one, two) = (platform(), platform());
    let script = one.compile(b"<?php function f() {} f();", "t.php").expect("compiles");
    let mut isolate = two.new_isolate();
    let mut host = Capture::default();
    isolate.new_context(&mut host).run(&script);
}

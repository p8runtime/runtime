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
    let script = platform.compile(source.as_bytes(), "t.php").expect("compiles");
    let mut isolate = platform.new_isolate();
    let mut host = Capture::default();
    let status = {
        let mut context = isolate.new_context(&mut host);
        if let Some(dir) = start {
            context.set_working_dir(dir);
        }
        context.run(&script)
    };
    (status, String::from_utf8(host.0).expect("utf-8"))
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

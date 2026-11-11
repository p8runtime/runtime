//! `php2 --compile <dir>` and a run with its package (#381): the run
//! prints what a run without it prints; an edited or new file runs its new
//! code; a package cut short or damaged is not used (the run says so on
//! stderr and compiles from source); a copy of php2 baked with the
//! package (`--bake`) uses it with no flag. That a run takes its code from
//! the package shows with `--package-check trust`, which by design runs
//! an edited file as packaged.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A project: main.php includes lib.php and every plugins/*.php.
fn project(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli_package").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("plugins")).expect("test dir");
    std::fs::write(
        dir.join("main.php"),
        "<?php require __DIR__ . '/lib.php';\nforeach (glob(__DIR__ . '/plugins/*.php') as $p) { include $p; }\necho greet('p8'), ' ', eval('return 6 * 7;'), ' ', $argv[1] ?? '-', \"\\n\";\n",
    )
    .expect("main.php");
    std::fs::write(dir.join("lib.php"), "<?php function greet($who) { return \"hello $who\"; }\n").expect("lib.php");
    std::fs::write(dir.join("plugins/one.php"), "<?php echo 'one ';").expect("one.php");
    dir.canonicalize().expect("canonical")
}

fn php2(exe: &Path, args: &[&str]) -> Output {
    Command::new(exe).args(args).output().expect("run")
}

fn text(out: &Output) -> (String, String) {
    (String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

fn compile(dir: &Path, extra: &[&str]) -> PathBuf {
    let exe = Path::new(env!("CARGO_BIN_EXE_php2"));
    let out = php2(exe, &[&["--compile", dir.to_str().expect("utf-8")][..], extra].concat());
    let (stdout, stderr) = text(&out);
    assert!(out.status.success() && stdout.starts_with("3 files compiled into "), "{stdout}{stderr}");
    dir.join(".p8/package")
}

#[test]
fn a_packaged_run_prints_what_a_run_without_prints() {
    let dir = project("same");
    let exe = Path::new(env!("CARGO_BIN_EXE_php2"));
    let main = dir.join("main.php");
    let main = main.to_str().expect("utf-8");
    let plain = php2(exe, &[main, "x"]);
    assert_eq!(text(&plain), ("one hello p8 42 x\n".to_string(), String::new()));
    let package = compile(&dir, &[]);
    let package = package.to_str().expect("utf-8");
    for check in ["hash", "stat", "trust"] {
        let packaged = php2(exe, &["--package", package, "--package-check", check, main, "x"]);
        assert_eq!(text(&packaged), text(&plain), "{check}");
    }
    // Trust shows the code comes from the package: an edit is not seen.
    std::fs::write(dir.join("lib.php"), "<?php function greet($who) { return \"howdy $who\"; }\n").expect("edit");
    assert_eq!(text(&php2(exe, &["--package", package, "--package-check", "trust", main, "x"])).0, "one hello p8 42 x\n");
    // The default (hash) and stat run the edited file, and a new one.
    std::fs::write(dir.join("plugins/two.php"), "<?php echo 'two ';").expect("two.php");
    for check in ["hash", "stat"] {
        assert_eq!(text(&php2(exe, &["--package", package, "--package-check", check, main, "x"])), ("one two howdy p8 42 x\n".to_string(), String::new()), "{check}");
    }
}

#[test]
fn a_cut_or_damaged_package_is_not_used() {
    let dir = project("damaged");
    let exe = Path::new(env!("CARGO_BIN_EXE_php2"));
    let package = compile(&dir, &[]);
    let main = dir.join("main.php");
    let bytes = std::fs::read(&package).expect("package");
    // Cut short: refused whole.
    let cut = dir.join("cut");
    std::fs::write(&cut, &bytes[..bytes.len() - 10]).expect("cut");
    let run = php2(exe, &["--package", cut.to_str().expect("utf-8"), "--package-check", "trust", main.to_str().expect("utf-8")]);
    assert_eq!(text(&run), ("one hello p8 42 -\n".to_string(), format!("php2: not using the package {}: the package is damaged\n", cut.display())));
    // A changed byte in the last file's code, just before the index (whose
    // start the header gives at byte 40) (trust: else it would be used):
    // that file is compiled from its source.
    std::fs::write(dir.join("plugins/one.php"), "<?php echo 'uno ';").expect("edit one.php");
    let mut changed = bytes.clone();
    let index_at = u64::from_le_bytes(changed[40..48].try_into().expect("8 bytes")) as usize;
    changed[index_at - 2] ^= 1;
    let damaged = dir.join("damaged");
    std::fs::write(&damaged, &changed).expect("damage");
    let run = php2(exe, &["--package", damaged.to_str().expect("utf-8"), "--package-check", "trust", main.to_str().expect("utf-8")]);
    assert_eq!(text(&run), ("uno hello p8 42 -\n".to_string(), String::new()), "plugins/one.php is the last file: its code is damaged, so it runs from source");
    // Not a package at all.
    let run = php2(exe, &["--package", main.to_str().expect("utf-8"), main.to_str().expect("utf-8")]);
    assert_eq!(text(&run).1, format!("php2: not using the package {}: not a p8 package\n", main.display()));
}

#[test]
fn a_baked_copy_of_php2_uses_its_package() {
    let dir = project("baked");
    let app = dir.join("app");
    compile(&dir, &["--bake", app.to_str().expect("utf-8")]);
    let main = dir.join("main.php");
    let main = main.to_str().expect("utf-8");
    assert_eq!(text(&php2(&app, &[main, "y"])), ("one hello p8 42 y\n".to_string(), String::new()));
    std::fs::write(dir.join("lib.php"), "<?php function greet($who) { return \"howdy $who\"; }\n").expect("edit");
    // Its package by default checks each file: the edit runs.
    assert_eq!(text(&php2(&app, &[main, "y"])).0, "one howdy p8 42 y\n");
    // Trusting it shows the code is the baked package's.
    assert_eq!(text(&php2(&app, &["--package-check", "trust", main, "y"])).0, "one hello p8 42 y\n");
    // Baked again from the baked copy: its old package is replaced, not kept.
    let again = dir.join("again");
    let out = php2(&app, &["--compile", dir.to_str().expect("utf-8"), "--bake", again.to_str().expect("utf-8")]);
    assert!(out.status.success(), "{:?}", text(&out));
    assert_eq!(std::fs::metadata(&again).expect("again").len(), std::fs::metadata(&app).expect("app").len());
    assert_eq!(text(&php2(&again, &["--package-check", "trust", main, "y"])).0, "one howdy p8 42 y\n");
}

#[test]
fn compile_is_an_option_so_a_file_named_compile_still_runs() {
    let dir = project("named_compile");
    std::fs::write(dir.join("compile"), "<?php echo 'ran compile';").expect("compile");
    let out = Command::new(env!("CARGO_BIN_EXE_php2")).current_dir(&dir).arg("compile").output().expect("run");
    assert_eq!(text(&out), ("ran compile".to_string(), String::new()));
}

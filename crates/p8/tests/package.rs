//! A package of compiled files (#381): a run that has one parses and
//! compiles none of the files in it (counted: `Platform::compiled_files`),
//! prints what a run without it prints, and never runs stale code: a
//! changed file, a new file and eval()'d code compile from their source.

use p8::{PackageCheck, Platform};
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Capture(Vec<u8>);

impl p8::Host for Capture {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
}

/// A project in a fresh folder: main.php includes lib.php (a function and
/// a class) and every plugins/*.php, and evals code.
fn project(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("package").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("plugins")).expect("test dir");
    std::fs::write(
        dir.join("main.php"),
        "<?php require __DIR__ . '/lib.php';\n\
         foreach (glob(__DIR__ . '/plugins/*.php') as $p) { include $p; }\n\
         echo greet('p8'), ' ', (new Counter)->add(2)->add(3)->n, ' ', eval('return 6 * 7;'), ' ', basename(__FILE__), ':', __LINE__, \"\\n\";\n",
    )
    .expect("main.php");
    std::fs::write(
        dir.join("lib.php"),
        "<?php function greet(string $who): string { return \"hello $who\"; }\n\
         class Counter { public int $n = 0; function add(int $k): static { $this->n += $k; return $this; } }\n",
    )
    .expect("lib.php");
    std::fs::write(dir.join("plugins/one.php"), "<?php echo 'one ';").expect("one.php");
    dir.canonicalize().expect("canonical")
}

fn platform() -> Platform {
    Platform::new(&[p8::extensions::STANDARD]).keep_compiled()
}

/// One request of `dir`/main.php on `platform`: its output, and how many
/// files it compiled from source.
fn run(platform: &Platform, dir: &Path) -> (String, usize) {
    let main = dir.join("main.php");
    let source = std::fs::read(&main).expect("main.php");
    let before = platform.compiled_files();
    let script = platform.compile(&source, main.to_str().expect("utf-8 path")).expect("compiles");
    let mut isolate = platform.new_isolate();
    let mut host = Capture::default();
    let status = isolate.new_context(&mut host).run(&script);
    assert_eq!(status, 0, "{}", String::from_utf8_lossy(&host.0));
    (String::from_utf8(host.0).expect("utf-8"), platform.compiled_files() - before)
}

/// `dir` compiled into a package, opened with `check`.
fn packaged(dir: &Path, check: PackageCheck) -> Platform {
    let out = dir.join(".p8/package");
    let platform = platform();
    let packed = platform.write_package(dir, &out).expect("writes the package");
    assert!(packed.failed.is_empty(), "{:?}", packed.failed);
    let package = platform.open_package(&out, check).expect("opens");
    platform.with_package(package)
}

const OUT: &str = "one hello p8 5 42 main.php:3\n";

#[test]
fn a_packaged_run_compiles_nothing() {
    let dir = project("compiles_nothing");
    assert_eq!(run(&platform(), &dir), (OUT.to_string(), 3), "without a package: main, lib and one compiled");
    let out = dir.join(".p8/package");
    let packed = platform().write_package(&dir, &out).expect("writes the package");
    assert_eq!(packed.files, 3);
    for check in [PackageCheck::Hash, PackageCheck::Stat, PackageCheck::Trust] {
        let base = platform();
        let package = base.open_package(&out, check).expect("opens");
        assert_eq!(package.len(), 3);
        assert_eq!(run(&base.with_package(package), &dir), (OUT.to_string(), 0), "{check:?}");
    }
}

#[test]
fn a_changed_file_runs_its_new_code() {
    for check in [PackageCheck::Hash, PackageCheck::Stat] {
        let dir = project(&format!("changed_{check:?}"));
        let platform = packaged(&dir, check);
        // The same length, so only the source itself tells them apart.
        let lib = std::fs::read_to_string(dir.join("lib.php")).expect("lib.php");
        std::fs::write(dir.join("lib.php"), lib.replace("hello", "howdy")).expect("edit lib.php");
        assert_eq!(run(&platform, &dir), (OUT.replace("hello", "howdy"), 1), "{check:?}: lib.php compiled again");
        assert_eq!(run(&platform, &dir), (OUT.replace("hello", "howdy"), 0), "{check:?}: and kept");
    }
}

#[test]
fn a_new_file_compiles_from_its_source() {
    let dir = project("new_file");
    let platform = packaged(&dir, PackageCheck::Hash);
    std::fs::write(dir.join("plugins/two.php"), "<?php echo 'two ';").expect("two.php");
    assert_eq!(run(&platform, &dir), (format!("one two {}", &OUT[4..]), 1));
}

#[test]
fn a_trusted_package_runs_as_packaged() {
    // What Trust means (opcache.validate_timestamps=0): the package is the
    // code, so an edit shows only once the package is written again.
    let dir = project("trusted");
    let platform = packaged(&dir, PackageCheck::Trust);
    let lib = std::fs::read_to_string(dir.join("lib.php")).expect("lib.php");
    std::fs::write(dir.join("lib.php"), lib.replace("hello", "howdy")).expect("edit lib.php");
    assert_eq!(run(&platform, &dir), (OUT.to_string(), 0));
    assert_eq!(run(&packaged(&dir, PackageCheck::Trust), &dir), (OUT.replace("hello", "howdy"), 0));
}

#[test]
fn a_damaged_or_cut_package_is_not_opened() {
    let dir = project("damaged");
    let out = dir.join(".p8/package");
    platform().write_package(&dir, &out).expect("writes the package");
    let bytes = std::fs::read(&out).expect("package");
    let cut = dir.join("cut");
    std::fs::write(&cut, &bytes[..bytes.len() - 1]).expect("cut");
    assert!(matches!(platform().open_package(&cut, PackageCheck::Hash), Err(p8::PackageError::Damaged)));
    let mut changed = bytes.clone();
    changed[70] ^= 1;
    let damaged = dir.join("damaged");
    std::fs::write(&damaged, &changed).expect("damage");
    assert!(matches!(platform().open_package(&damaged, PackageCheck::Hash), Err(p8::PackageError::Damaged)));
    assert!(matches!(platform().open_package(dir.join("main.php"), PackageCheck::Hash), Err(p8::PackageError::NotAPackage)));
    // A package for other extensions is another engine's.
    let other = Platform::new(&[p8::extensions::STANDARD, p8::extensions::XML]);
    assert!(matches!(other.open_package(&out, PackageCheck::Hash), Err(p8::PackageError::OtherEngine)));
}

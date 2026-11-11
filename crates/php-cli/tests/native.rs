//! `php2 --compile <dir> --native <exe>` (#402, Phase 4): an executable
//! holding the runtime, the package and the package's functions as
//! machine code. Its runs print what php2's print, byte for byte (and
//! what PHP 8.6 prints, recorded below); its native functions run (the
//! executable's `--native-counts` says how many were entered and how
//! many handed the frame back), and what they do not handle runs on the
//! interpreter, from the instruction they stopped at, with the state they
//! left (#388).
//!
//! It needs the runtime library beside php2: `cargo build --release
//! --locked -p p8-native` (the gates build it first).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A hot loop over ints and floats, a call into a native function from
/// a native one and from the interpreter, and a function that is not
/// native (a concatenation) called from the script.
const HOT: &str = r#"<?php
function sq($x) { return $x * $x; }
function sum_to($n) {
    $s = 0;
    for ($i = 0; $i < $n; $i++) {
        $s = $s + sq($i) - 1;
    }
    return $s;
}
function wave($n) {
    $x = 0.5;
    $k = 0;
    while ($k < $n) {
        $x = $x * 1.5 + 0.25;
        if ($x > 1000.0) { $x = $x - 999.0; }
        $k += 1;
    }
    return $x;
}
function label($n) { return "n=" . $n; }
echo sum_to(1000), "\n", wave(200), "\n", label(sum_to(10)), "\n";
for ($r = 0; $r < 3; $r++) { echo sum_to($r * 7), " "; }
echo "\n";
"#;

/// What PHP 8.6.0RC5 prints for HOT.
const HOT_OUT: &str = "332832500\n434.56709442529\nn=275\n0 85 805 \n";

/// Values the native code was not made for: strings where it adds ints,
/// an overflow to float (in a call, and in the middle of a loop, whose
/// state the interpreter takes over), arrays, null, a float.
const FALLBACK: &str = r#"<?php
function add($a, $b) { $c = $a + $b; return $c; }
function mix($n) {
    $t = 0;
    $x = 1;
    for ($i = 0; $i < $n; $i++) {
        $x = $x * 3;
        $t = $t + $i;
    }
    return $t * 1000 + $i;
}
var_dump(add(2, 3));
var_dump(add("4", "5"));
var_dump(add(PHP_INT_MAX, 1));
var_dump(add(1.5, 2));
var_dump(add([1], [2, 3]));
var_dump(add(null, 3));
var_dump(mix(50));
var_dump(mix(10));
"#;

/// What PHP 8.6.0RC5 prints for FALLBACK.
const FALLBACK_OUT: &str = "int(5)\nint(9)\nfloat(9.2233720368547758E+18)\nfloat(3.5)\narray(2) {\n  [0]=>\n  int(1)\n  [1]=>\n  int(3)\n}\nint(3)\narray(2) {\n  [0]=>\n  float(9.2233720368547758E+18)\n  [1]=>\n  int(5)\n}\narray(2) {\n  [0]=>\n  int(13)\n  [1]=>\n  int(3)\n}\n";

fn php2() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_php2"))
}

/// The runtime library php2 links native executables with.
fn host_library() -> PathBuf {
    let lib = php2().with_file_name("libp8_native.a");
    assert!(lib.is_file(), "{} is missing: build it with `cargo build --release --locked -p p8-native`", lib.display());
    lib
}

/// `files` in a fresh folder, compiled into a native executable there.
pub fn build(name: &str, files: &[(&str, &str)]) -> (PathBuf, PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cli_native").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).expect("test dir");
    for (file, text) in files {
        std::fs::write(dir.join("src").join(file), text).expect("fixture");
    }
    let dir = dir.canonicalize().expect("canonical");
    let exe = native(&dir.join("src"), &dir.join("package"), &dir.join("app"));
    (dir.join("src"), exe)
}

/// `php2 --compile src --package package --native exe`; the executable.
pub fn native(src: &Path, package: &Path, exe: &Path) -> PathBuf {
    let out = Command::new(php2())
        .arg("--compile")
        .arg(src)
        .arg("--package")
        .arg(package)
        .arg("--native")
        .arg(exe)
        .arg("--native-host")
        .arg(host_library())
        .output()
        .expect("run php2 --compile --native");
    let said = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && exe.is_file(), "{said}");
    exe.to_path_buf()
}

fn run(exe: &Path, args: &[&Path]) -> Output {
    Command::new(exe).args(args).output().expect("run")
}

/// The counts `--native-counts` printed: (entries, returns, guards).
fn counts(out: &Output) -> (u64, u64, u64) {
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().find(|l| l.starts_with("p8 native: ")).unwrap_or_else(|| panic!("no counts in {err:?}"));
    let n: Vec<u64> = line.split(|c: char| !c.is_ascii_digit()).filter(|w| !w.is_empty()).filter_map(|w| w.parse().ok()).collect();
    (n[1], n[2], n[3])
}

#[test]
fn a_native_executable_prints_what_php2_prints_and_runs_native_code() {
    let (src, exe) = build("hot", &[("hot.php", HOT)]);
    let script = src.join("hot.php");
    let plain = run(php2(), &[&script]);
    assert_eq!(String::from_utf8_lossy(&plain.stdout), HOT_OUT);
    let native = run(&exe, &[Path::new("--native-counts"), &script]);
    assert!(native.status.success(), "{}", String::from_utf8_lossy(&native.stderr));
    assert_eq!(native.stdout, plain.stdout);
    let (entries, returns, guards) = counts(&native);
    // sum_to 5 times from the script, sq once per round of it (1000 + 10
    // + 0 + 7 + 14), wave once: all native, every one returns.
    assert_eq!(entries, 5 + 1031 + 1, "native entries");
    assert_eq!((returns, guards), (entries, 0));
}

#[test]
fn what_native_code_does_not_handle_runs_on_the_interpreter_from_where_it_stopped() {
    let (src, exe) = build("fallback", &[("fallback.php", FALLBACK)]);
    let script = src.join("fallback.php");
    let plain = run(php2(), &[&script]);
    assert_eq!(String::from_utf8_lossy(&plain.stdout), FALLBACK_OUT);
    let native = run(&exe, &[Path::new("--native-counts"), &script]);
    assert_eq!(String::from_utf8_lossy(&native.stdout), FALLBACK_OUT, "{}", String::from_utf8_lossy(&native.stderr));
    let (entries, returns, guards) = counts(&native);
    // Eight native calls: three return natively (two ints, an int and a
    // float, mix(10)); the strings, the overflow, the arrays, null, and
    // mix(50)'s `$x * 3` overflowing in its 40th round (the loop goes on
    // on the interpreter, with `$t` and `$i` as the native code left
    // them) hand the frame back at a guard.
    assert_eq!((entries, returns, guards), (8, 3, 5));
}

#[test]
fn a_native_executable_runs_a_changed_file_from_its_source() {
    let (src, exe) = build("changed", &[("hot.php", HOT)]);
    let script = src.join("hot.php");
    std::fs::write(&script, "<?php function sq($x) { return $x + 1; }\necho sq(4), \"\\n\";\n").expect("edit");
    let native = run(&exe, &[Path::new("--native-counts"), &script]);
    assert_eq!(String::from_utf8_lossy(&native.stdout), "5\n");
    // Code compiled from source is never native.
    assert_eq!(counts(&native).0, 0);
}

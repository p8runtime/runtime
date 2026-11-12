//! `p8 compile [dir] --native <exe>` (#402, Phase 4): an executable
//! holding the runtime, the package and the package's functions as
//! machine code. Its runs print what p8's print, byte for byte (and
//! what PHP 8.6 prints, recorded below); its native functions run (the
//! executable's `--native-counts` says how many were entered and how
//! many handed the frame back), and what they do not handle runs on the
//! interpreter, from the instruction they stopped at, with the state they
//! left (#388).
//!
//! It needs the runtime library beside p8: `cargo build --release
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
const HOT_OUT: &str = "332832500\n205.99228973386\nn=275\n0 84 805 \n";

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
// (A function's first call resolves it on the interpreter: these make
// every call below a native one.)
add(0, 0);
mix(1);
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
const FALLBACK_OUT: &str = "int(5)\nint(9)\nfloat(9.223372036854776E+18)\nfloat(3.5)\narray(2) {\n  [0]=>\n  int(1)\n  [1]=>\n  int(3)\n}\nint(3)\nint(1225050)\nint(45010)\n";

fn p8() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_p8"))
}

/// The runtime library p8 links native executables with.
fn host_library() -> PathBuf {
    let lib = p8().with_file_name("libp8_native.a");
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

/// `p8 compile src --package package --native exe`; the executable.
pub fn native(src: &Path, package: &Path, exe: &Path) -> PathBuf {
    let out = Command::new(p8())
        .arg("compile")
        .arg(src)
        .arg("--package")
        .arg(package)
        .arg("--native")
        .arg(exe)
        .arg("--native-host")
        .arg(host_library())
        .output()
        .expect("run p8 compile --native");
    let said = String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success() && exe.is_file(), "{said}");
    exe.to_path_buf()
}

fn run(exe: &Path, args: &[&Path]) -> Output {
    Command::new(exe).args(args).output().expect("run")
}

/// The counts `--native-counts` printed: (entries, returns, guards).
fn counts(out: &Output) -> (u64, u64, u64) {
    let c = all_counts(out);
    (c.entries, c.returns, c.guards)
}

/// What `--native-counts` printed.
#[derive(Debug, PartialEq)]
struct Counts {
    entries: u64,
    returns: u64,
    /// Frames handed back at an instruction (the interpreter ran the rest).
    guards: u64,
    /// Frames an instruction the interpreter ran left (an exception).
    lefts: u64,
    /// Instructions the interpreter ran for the native code.
    helps: u64,
}

fn all_counts(out: &Output) -> Counts {
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().find_map(|l| l.strip_prefix("p8 native: ")).unwrap_or_else(|| panic!("no counts in {err:?}"));
    let n: Vec<u64> = line.split(|c: char| !c.is_ascii_digit()).filter(|w| !w.is_empty()).filter_map(|w| w.parse().ok()).collect();
    Counts { entries: n[1], returns: n[2], guards: n[3], lefts: n[4], helps: n[5] }
}

/// `script` (a file of its own) through a native executable: its output
/// (equal to p8's) and the counts.
fn native_run(name: &str, script: &str) -> (String, Counts) {
    let (src, exe) = build(name, &[("main.php", script)]);
    let main = src.join("main.php");
    let plain = run(p8(), &[&main]);
    let native = run(&exe, &[Path::new("--native-counts"), &main]);
    assert!(native.status.success(), "{name}: {:?} {}", native.status, String::from_utf8_lossy(&native.stderr));
    assert_eq!(String::from_utf8_lossy(&native.stdout), String::from_utf8_lossy(&plain.stdout), "{name}");
    (String::from_utf8_lossy(&native.stdout).into_owned(), all_counts(&native))
}

/// Native code is entered on every path a user function is called by:
/// its first call (which resolves the site on the interpreter), a method,
/// a static method, a closure, a callback a builtin makes, a call from
/// native code. Each script calls its function the number of times
/// counted, from the main code (never native).
#[test]
fn native_code_is_entered_on_every_call_path() {
    let paths = [
        ("first_call", "<?php function f($x) { return $x + 1; }\necho f(1), \"\\n\";\n", "2\n", 1),
        ("method", "<?php class A { function m($x) { return $x * 2; } }\n$a = new A;\necho $a->m(4), \"\\n\";\n", "8\n", 1),
        ("static_method", "<?php class A { static function s($x) { return $x - 1; } }\necho A::s(4), \"\\n\";\n", "3\n", 1),
        ("closure", "<?php $f = function ($x) { return $x + 10; };\necho $f(1), \"\\n\";\n", "11\n", 1),
        ("callback", "<?php function dbl($x) { return $x * 2; }\necho implode(',', array_map('dbl', [1, 2, 3])), \"\\n\";\n", "2,4,6\n", 3),
        ("from_native", "<?php function inner($x) { return $x + 1; }\nfunction outer($x) { return inner($x) + inner($x); }\necho outer(1), \"\\n\";\n", "4\n", 3),
    ];
    for (name, script, out, calls) in paths {
        let (got, c) = native_run(name, script);
        assert_eq!(got, out, "{name}");
        assert_eq!((c.entries, c.returns, c.guards, c.lefts), (calls, calls, 0, 0), "{name}: {c:?}");
    }
}

/// An instruction with no template (a concatenation), and a value holding
/// memory (a string copied and released), no longer end the native run:
/// the interpreter runs that one instruction and the native code goes on
/// at the next pc. The loop runs to its return in native code.
#[test]
fn an_instruction_without_a_template_runs_on_the_interpreter_and_the_native_code_goes_on() {
    let script = "<?php function grow($n) { $s = ''; for ($i = 0; $i < $n; $i++) { $s .= 'ab'; $t = $s; } return strlen($t) + $i; }\necho grow(50), \"\\n\";\n";
    let (got, c) = native_run("helped", script);
    assert_eq!(got, "150\n");
    assert_eq!((c.entries, c.returns, c.guards, c.lefts), (1, 1, 0, 0), "{c:?}");
    // The concatenation and the string's copy, each round, and strlen: the
    // interpreter's.
    assert!(c.helps >= 100, "{c:?}");
}

/// An executable whose machine code another build of p8 made (here: the
/// build identity in its image changed) is not run by this runtime
/// library: the image is stamped with the engine key of the p8 that made
/// it, and a runtime library runs only its own.
#[test]
fn machine_code_of_another_build_is_refused() {
    let (src, exe) = build("other_build", &[("main.php", "<?php function f() { return 1; }\necho f(), \"\\n\";\n")]);
    let main = src.join("main.php");
    assert_eq!(String::from_utf8_lossy(&run(&exe, &[&main]).stdout), "1\n");
    // The image: `p8native`, its version (2), then at 56 the engine key.
    let mut bytes = std::fs::read(&exe).expect("the executable");
    let mut magic = b"p8native".to_vec();
    magic.extend_from_slice(&2u64.to_le_bytes());
    let at = bytes.windows(16).position(|w| w == magic.as_slice()).expect("the image");
    bytes[at + 56] ^= 0xff;
    let other = exe.with_file_name("other");
    std::fs::write(&other, &bytes).expect("write");
    std::fs::set_permissions(&other, std::os::unix::fs::PermissionsExt::from_mode(0o755)).expect("chmod");
    // (macOS runs a changed executable only signed again; ad hoc, as the
    // linker signed it.)
    if cfg!(target_os = "macos") {
        let signed = Command::new("codesign").args(["-s", "-", "-f"]).arg(&other).output().expect("codesign");
        assert!(signed.status.success(), "{}", String::from_utf8_lossy(&signed.stderr));
    }
    let refused = run(&other, &[&main]);
    assert_eq!(refused.status.code(), Some(70), "{}", String::from_utf8_lossy(&refused.stderr));
    assert!(refused.stdout.is_empty());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("another build of p8"));
}

/// The request's working directory (#474) reaches the builtins native
/// code calls: they run through the interpreter's own arms, on the same
/// request. A native function that `chdir`s reads a relative path there.
#[test]
fn the_working_directory_reaches_builtins_called_from_native_code() {
    let script = "<?php function read_in($dir) { chdir($dir); return file_get_contents('x') . getcwd(); }\nmkdir(__DIR__ . '/sub');\nfile_put_contents(__DIR__ . '/sub/x', 'in sub ');\necho read_in(__DIR__ . '/sub') === 'in sub ' . realpath(__DIR__ . '/sub') ? \"ok\\n\" : \"wrong\\n\";\n";
    let (src, exe) = build("cwd", &[("main.php", script)]);
    let main = src.join("main.php");
    let native = run(&exe, &[Path::new("--native-counts"), &main]);
    assert_eq!(String::from_utf8_lossy(&native.stdout), "ok\n", "{}", String::from_utf8_lossy(&native.stderr));
    let c = all_counts(&native);
    assert_eq!((c.entries, c.returns, c.lefts), (1, 1, 0), "{c:?}");
}

/// Typed parameters and returns of scalar types are checked inline (by
/// the value's tag): no instruction of this function is the interpreter's.
#[test]
fn scalar_types_are_checked_inline() {
    let script = "<?php function add(int $a, float $b): float { return $a + $b; }\necho add(1, 2.5), \"\\n\";\n";
    let (got, c) = native_run("typed", script);
    assert_eq!(got, "3.5\n");
    assert_eq!((c.entries, c.returns, c.guards, c.lefts, c.helps), (1, 1, 0, 0, 0), "{c:?}");
}

/// An exception thrown in native code: caught in the same function (the
/// native code goes on at the handler), and thrown out of one (the frame
/// is left; the caller catches it).
#[test]
fn exceptions_in_native_code_go_where_the_interpreter_sends_them() {
    let script = "<?php function f($x) { try { if ($x) { throw new Exception('in'); } return 1; } catch (Exception $e) { return 2; } }\nfunction g() { throw new Exception('out'); }\necho f(0), f(1), \"\\n\";\ntry { g(); } catch (Exception $e) { echo $e->getMessage(), \"\\n\"; }\n";
    let (got, c) = native_run("exceptions", script);
    assert_eq!(got, "12\nout\n");
    assert_eq!((c.entries, c.returns, c.lefts), (3, 2, 1), "{c:?}");
}

#[test]
fn a_native_executable_prints_what_p8_prints_and_runs_native_code() {
    let (src, exe) = build("hot", &[("hot.php", HOT)]);
    let script = src.join("hot.php");
    let plain = run(p8(), &[&script]);
    assert_eq!(String::from_utf8_lossy(&plain.stdout), HOT_OUT);
    let native = run(&exe, &[Path::new("--native-counts"), &script]);
    assert!(native.status.success(), "{}", String::from_utf8_lossy(&native.stderr));
    assert_eq!(native.stdout, plain.stdout);
    let (entries, returns, guards) = counts(&native);
    // sq runs once per round of sum_to (1031 rounds): all native but a
    // function's first call (which resolves it on the interpreter), every
    // one returns.
    assert!(entries >= 1031, "{entries} native entries");
    assert_eq!((returns, guards), (entries, 0));
}

#[test]
fn what_native_code_does_not_handle_runs_on_the_interpreter_from_where_it_stopped() {
    let (src, exe) = build("fallback", &[("fallback.php", FALLBACK)]);
    let script = src.join("fallback.php");
    let plain = run(p8(), &[&script]);
    assert_eq!(String::from_utf8_lossy(&plain.stdout), FALLBACK_OUT);
    let native = run(&exe, &[Path::new("--native-counts"), &script]);
    assert_eq!(String::from_utf8_lossy(&native.stdout), FALLBACK_OUT, "{}", String::from_utf8_lossy(&native.stderr));
    let c = all_counts(&native);
    // Every call is native and returns natively: the strings, the
    // overflow, the arrays, null, and mix(50)'s `$x * 3` overflowing in
    // its 40th round are each one instruction the interpreter runs, with
    // the registers as the native code left them, the native code going
    // on after it (the loop's `$t` and `$i` are the native code's).
    assert!(c.entries >= 8, "{c:?}");
    assert_eq!((c.returns, c.guards, c.lefts), (c.entries, 0, 0), "{c:?}");
    assert!(c.helps >= 5, "{c:?}");
}

/// A native function calling itself a million deep: each native call runs
/// its callee nested, on the native stack, until nested runs reach their
/// limit; then the interpreter makes the calls, its frames on the heap
/// (#444). Without that, the thread's stack ran out (SIGBUS).
#[test]
fn a_deep_recursion_through_native_code_runs_as_on_the_interpreter() {
    const DEEP: &str = "<?php\nfunction depth($n) { if ($n == 0) { return 0; } return depth($n - 1) + 1; }\ndepth(1);\nforeach ([1000, 50000, 200000, 1000000] as $n) { echo $n, \" => \", depth($n), \"\\n\"; }\n";
    let (src, exe) = build("deep", &[("deep.php", DEEP)]);
    let script = src.join("deep.php");
    let want = "1000 => 1000\n50000 => 50000\n200000 => 200000\n1000000 => 1000000\n";
    let plain = run(p8(), &[&script]);
    assert_eq!(String::from_utf8_lossy(&plain.stdout), want);
    let native = run(&exe, &[Path::new("--native-counts"), &script]);
    assert!(native.status.success(), "{:?}: {}", native.status, String::from_utf8_lossy(&native.stderr));
    assert_eq!(native.stdout, plain.stdout);
    // (It ran natively: the depth function is native code.)
    assert!(counts(&native).0 > 1000);
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

//! p8 end to end through the p8 host: each tests/scripts/NAME.php must print exactly
//! what real PHP printed for it (NAME.out, recorded reading the script from
//! stdin, its file name replaced by %s). Recorded with php:8.4-cli; p8's
//! target is PHP 8.6 since 4 Nov 2026, and a script is re-recorded with 8.6
//! when p8 does what 8.6 does for it (#279).

use std::path::Path;
use std::process::Command;

/// The scripts write fixed temporary files (zlib.php: p8zlib.gz), so the
/// two runs of them take turns.
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

/// Every tests/scripts/NAME.php run with `options` prints NAME.out.
fn check(options: &[&std::ffi::OsStr]) {
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
        let out = Command::new(env!("CARGO_BIN_EXE_p8")).args(options).arg(script).output().expect("run p8");
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

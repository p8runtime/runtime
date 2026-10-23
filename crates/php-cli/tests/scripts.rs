//! p8 end to end through the php2 host: each tests/scripts/NAME.php must print exactly
//! what PHP 8.4 printed for it (NAME.out, recorded with php:8.4-cli reading
//! the script from stdin, its file name replaced by %s).

use std::path::Path;
use std::process::Command;

#[test]
fn scripts_match_php_8_4() {
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
        let want = std::fs::read_to_string(script.with_extension("out")).expect("expected output");
        let out = Command::new(env!("CARGO_BIN_EXE_php2")).arg(script).output().expect("run php2");
        let shown = std::fs::canonicalize(script).expect("canonical path");
        let got = String::from_utf8_lossy(&out.stdout).replace(&*shown.to_string_lossy(), "%s");
        if got != want {
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

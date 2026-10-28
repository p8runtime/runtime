//! mysqli against a real server: tests/mysqli/NAME.php must print what PHP
//! 8.4 (php:8.4-cli with mysqli, mysqlnd) printed for it against MariaDB
//! 11.4, with user `wp`, password `wp` and database `wp`. Ignored unless
//! asked for, since it needs that server:
//!
//!     P8_MYSQL=127.0.0.1:3307 cargo test -p php-cli --test mysqli -- --ignored
//!
//! The scripts read the server's address from DBHOST and DBPORT, and never
//! print it.

use std::path::Path;
use std::process::Command;

#[test]
#[ignore = "needs a MariaDB server; set P8_MYSQL=host:port and pass --ignored"]
fn mysqli_matches_php_8_4() {
    let addr = std::env::var("P8_MYSQL").expect("P8_MYSQL=host:port of the MariaDB server");
    let (host, port) = addr.rsplit_once(':').expect("P8_MYSQL is host:port");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mysqli");
    let mut scripts: Vec<_> = std::fs::read_dir(&dir)
        .expect("tests/mysqli")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "php"))
        .collect();
    scripts.sort();
    assert!(!scripts.is_empty(), "no scripts in {}", dir.display());
    for script in &scripts {
        let want = std::fs::read_to_string(script.with_extension("out")).expect("expected output");
        let out = Command::new(env!("CARGO_BIN_EXE_php2")).arg(script).env("DBHOST", host).env("DBPORT", port).output().expect("run php2");
        let shown = std::fs::canonicalize(script).expect("canonical path");
        let got = String::from_utf8_lossy(&out.stdout).replace(&*shown.to_string_lossy(), "%s");
        if let Some(line) = got.lines().zip(want.lines()).position(|(g, w)| g != w).or((got.lines().count() != want.lines().count()).then(|| got.lines().count().min(want.lines().count()))) {
            panic!("{}: first difference at line {}\n  got:  {:?}\n  want: {:?}", script.display(), line + 1, got.lines().nth(line), want.lines().nth(line));
        }
    }
}

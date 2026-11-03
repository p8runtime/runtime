//! pdo_mysql against a real server: tests/pdo_mysql/NAME.php must print
//! what PHP 8.4 (pdo_mysql on mysqlnd) printed for it against MariaDB 11.4,
//! with user `wp`, password `wp` and a database `pdo_p8` they may use.
//! Ignored unless asked for, since it needs that server:
//!
//!     docker exec p8-mariadb mariadb -uroot -pp8root -e "CREATE DATABASE IF NOT EXISTS pdo_p8; GRANT ALL ON pdo_p8.* TO 'wp'@'%'"
//!     P8_MYSQL=127.0.0.1:3307 cargo test -p php-cli --test pdo_mysql -- --ignored
//!
//! The scripts read the server's address from DBHOST and DBPORT, and never
//! print it.

use std::path::Path;
use std::process::Command;

#[test]
#[ignore = "needs a MariaDB server; set P8_MYSQL=host:port and pass --ignored"]
fn pdo_mysql_matches_php_8_4() {
    let addr = std::env::var("P8_MYSQL").expect("P8_MYSQL=host:port of the MariaDB server");
    let (host, port) = addr.rsplit_once(':').expect("P8_MYSQL is host:port");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pdo_mysql");
    let mut scripts: Vec<_> = std::fs::read_dir(&dir)
        .expect("tests/pdo_mysql")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "php"))
        .collect();
    scripts.sort();
    assert!(!scripts.is_empty(), "no scripts in {}", dir.display());
    let mut failures = Vec::new();
    for script in &scripts {
        let want = String::from_utf8_lossy(&std::fs::read(script.with_extension("out")).expect("expected output")).into_owned();
        let out = Command::new(env!("CARGO_BIN_EXE_php2")).arg(script).env("DBHOST", host).env("DBPORT", port).output().expect("run php2");
        let shown = std::fs::canonicalize(script).expect("canonical path");
        let got = String::from_utf8_lossy(&out.stdout).replace(&*shown.to_string_lossy(), "%s");
        if let Some(line) = got.lines().zip(want.lines()).position(|(g, w)| g != w).or((got.lines().count() != want.lines().count()).then(|| got.lines().count().min(want.lines().count()))) {
            failures.push(format!("{}: first difference at line {}\n  got:  {:?}\n  want: {:?}", script.display(), line + 1, got.lines().nth(line), want.lines().nth(line)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

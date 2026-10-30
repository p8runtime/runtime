//! pdo_pgsql against a real server: tests/pdo_pgsql/NAME.php must print
//! what PHP 8.4 (pdo_pgsql on libpq) printed for it against PostgreSQL 17,
//! with user `postgres`, password `p8` and database `test`. Ignored unless
//! asked for, since it needs that server:
//!
//!     docker run -d --name p8-pg -e POSTGRES_PASSWORD=p8 -e POSTGRES_DB=test -p 127.0.0.1:55432:5432 postgres:17-bookworm
//!     P8_PGSQL=127.0.0.1:55432 cargo test -p php-cli --test pdo_pgsql -- --ignored
//!
//! The scripts read the server's address from DBHOST and DBPORT, and never
//! print it.

use std::path::Path;
use std::process::Command;

#[test]
#[ignore = "needs a PostgreSQL server; set P8_PGSQL=host:port and pass --ignored"]
fn pdo_pgsql_matches_php_8_4() {
    let addr = std::env::var("P8_PGSQL").expect("P8_PGSQL=host:port of the PostgreSQL server");
    let (host, port) = addr.rsplit_once(':').expect("P8_PGSQL is host:port");
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/pdo_pgsql");
    let mut scripts: Vec<_> = std::fs::read_dir(&dir)
        .expect("tests/pdo_pgsql")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "php"))
        .collect();
    scripts.sort();
    assert!(!scripts.is_empty(), "no scripts in {}", dir.display());
    let mut failures = Vec::new();
    for script in &scripts {
        let want = std::fs::read_to_string(script.with_extension("out")).expect("expected output");
        let out = Command::new(env!("CARGO_BIN_EXE_php2")).arg(script).env("DBHOST", host).env("DBPORT", port).output().expect("run php2");
        let shown = std::fs::canonicalize(script).expect("canonical path");
        let got = String::from_utf8_lossy(&out.stdout).replace(&*shown.to_string_lossy(), "%s");
        if let Some(line) = got.lines().zip(want.lines()).position(|(g, w)| g != w).or((got.lines().count() != want.lines().count()).then(|| got.lines().count().min(want.lines().count()))) {
            failures.push(format!("{}: first difference at line {}\n  got:  {:?}\n  want: {:?}", script.display(), line + 1, got.lines().nth(line), want.lines().nth(line)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

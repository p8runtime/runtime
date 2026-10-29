//! max_execution_time: a script that runs past it ends with PHP's fatal
//! error (as php:8.4-cli prints it) and status 255, in about that time.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn max_execution_time_stops_a_loop() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/time_limit/loop.php");
    let started = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_php2")).args(["-d", "max_execution_time=1"]).arg(&script).output().expect("run php2");
    let took = started.elapsed();
    let shown = std::fs::canonicalize(&script).expect("canonical path");
    let got = String::from_utf8_lossy(&out.stdout).replace(&*shown.to_string_lossy(), "%s");
    assert_eq!(got, "\nFatal error: Maximum execution time of 1 second exceeded in %s on line 5\n");
    assert_eq!(out.status.code(), Some(255));
    assert!(took >= Duration::from_secs(1) && took < Duration::from_secs(10), "stopped after {took:?}");
}

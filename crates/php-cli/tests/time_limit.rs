//! max_execution_time: a script that runs past it ends with PHP's fatal
//! error (as PHP 8.6 prints it) and status 255, in about that time.

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn max_execution_time_stops_a_loop() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/time_limit/loop.php");
    let started = Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_p8")).args(["-d", "max_execution_time=1"]).arg(&script).output().expect("run p8");
    let took = started.elapsed();
    let shown = std::fs::canonicalize(&script).expect("canonical path");
    let got = String::from_utf8_lossy(&out.stdout).replace(&*shown.to_string_lossy(), "%s");
    // With its stack trace (fatal_error_backtraces, PHP 8.5+).
    assert_eq!(got, "\nFatal error: Maximum execution time of 1 second exceeded in %s on line 5\nStack trace:\n#0 {main}\n");
    assert_eq!(out.status.code(), Some(255));
    assert!(took >= Duration::from_secs(1) && took < Duration::from_secs(10), "stopped after {took:?}");
}

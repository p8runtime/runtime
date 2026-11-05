//! Output reaches fd 1 as it is written, as in PHP's CLI: echo, STDOUT and
//! php://stdout keep their order with STDERR when both go to one pipe
//! (#305: they used to wait in a 64 KB buffer until flush() or exit).

use std::io::Read;
use std::path::Path;
use std::process::Command;

#[test]
fn stdout_and_stderr_keep_their_order() {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/stdout_order/order.php");
    let (mut reader, writer) = std::io::pipe().expect("pipe");
    let mut child = Command::new(env!("CARGO_BIN_EXE_php2"))
        .arg(&script)
        .stdout(writer.try_clone().expect("clone pipe"))
        .stderr(writer)
        .spawn()
        .expect("run php2");
    let status = child.wait().expect("wait for php2");
    drop(child);
    let mut got = String::new();
    reader.read_to_string(&mut got).expect("read output");
    assert!(status.success(), "php2 exited with {status}");
    // What php:8.6-cli prints for the same script.
    assert_eq!(got, "1 STDOUT after fflush\n2 STDERR\n3 php://stdout after fflush\n4 STDERR\n5 echo\n6 STDERR\n7 echo without flush\n8 STDERR\n");
}

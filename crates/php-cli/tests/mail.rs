//! mail() through a sendmail that keeps what it is given: tests/mail/send.php
//! must print send.out, and hand the program (sendmail.sh, as
//! sendmail_path) exactly the bytes in send.mbox, as PHP 8.4 (php:8.4-cli)
//! did for it.

use std::path::Path;
use std::process::Command;

#[test]
fn mail_matches_php_8_4() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mail");
    let mailbox = Path::new(env!("CARGO_TARGET_TMPDIR")).join("mail.mbox");
    let _ = std::fs::remove_file(&mailbox);
    let script = dir.join("send.php");
    let out = Command::new(env!("CARGO_BIN_EXE_p8"))
        .arg("-d")
        .arg(format!("sendmail_path={}", dir.join("sendmail.sh").display()))
        .arg(&script)
        .env("MAILBOX", &mailbox)
        .output()
        .expect("run p8");
    let shown = std::fs::canonicalize(&script).expect("canonical path");
    let got = String::from_utf8_lossy(&out.stdout).replace(&*shown.to_string_lossy(), "%s");
    assert_eq!(got, std::fs::read_to_string(dir.join("send.out")).expect("send.out"));
    let sent = std::fs::read(&mailbox).expect("the mail sendmail.sh kept");
    assert_eq!(String::from_utf8_lossy(&sent), String::from_utf8_lossy(&std::fs::read(dir.join("send.mbox")).expect("send.mbox")));
}

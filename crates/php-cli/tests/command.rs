//! The workspace builds one command for users, `p8`, and php-src's test
//! runner, `php-test`. Any other binary (a second command beside p8, the
//! legacy engine's `php` or `php-fpm`) fails this.

use std::process::Command;

/// Every binary target in the workspace, by name, as `cargo metadata` lists them.
fn binaries() -> Vec<String> {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.toml");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let out = Command::new(cargo).args(["metadata", "--no-deps", "--format-version", "1", "--offline", "--manifest-path", root]).output().expect("cargo metadata");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let json = String::from_utf8(out.stdout).expect("utf-8");
    // A target is a flat object (no object inside it): the one around each
    // `"kind":["bin"]`, and its name.
    let mut names = Vec::new();
    let mut at = 0;
    while let Some(found) = json[at..].find(r#""kind":["bin"]"#) {
        let kind = at + found;
        let start = json[..kind].rfind('{').expect("the target's start");
        let end = kind + json[kind..].find('}').expect("the target's end");
        let target = &json[start..end];
        let name = target.split(r#""name":""#).nth(1).and_then(|s| s.split('"').next()).expect("the target's name");
        names.push(name.to_string());
        at = end;
    }
    names.sort();
    names
}

#[test]
fn p8_is_the_only_command() {
    assert_eq!(binaries(), ["p8", "php-test"]);
    // And it is the one this package builds.
    assert!(std::path::Path::new(env!("CARGO_BIN_EXE_p8")).is_file());
}

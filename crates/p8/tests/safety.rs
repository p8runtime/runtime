//! p8's safety rules, checked so they cannot erode silently: every p8 crate
//! forbids `unsafe`, and nothing p8 depends on builds or links C (stricter
//! than the legacy engine, which still links pcre2).

use std::path::Path;
use std::process::Command;

fn crates_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("crates/")
}

#[test]
fn every_p8_crate_forbids_unsafe() {
    let mut checked = 0;
    for entry in std::fs::read_dir(crates_dir()).expect("crates/") {
        let dir = entry.expect("entry").path();
        let name = dir.file_name().expect("name").to_string_lossy().into_owned();
        if name != "p8" && !name.starts_with("p8-") && !name.starts_with("php-") {
            continue;
        }
        let root = ["src/lib.rs", "src/main.rs"].iter().map(|f| dir.join(f)).find(|p| p.exists());
        let root = root.unwrap_or_else(|| panic!("{name}: no src/lib.rs or src/main.rs"));
        let source = std::fs::read_to_string(&root).expect("crate root");
        assert!(
            source.lines().any(|l| l.trim_start().starts_with("#![forbid(unsafe_code)]")),
            "{} does not forbid unsafe code",
            root.display()
        );
        checked += 1;
    }
    assert!(checked >= 8, "only {checked} crates checked");
}

#[test]
fn p8_links_no_c() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args(["tree", "-p", "p8", "-e", "normal", "--prefix", "none", "--format", "{p}"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo tree");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let tree = String::from_utf8_lossy(&out.stdout);
    let offenders: Vec<&str> = tree
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        // core-foundation-sys is Rust declarations of macOS's own framework (chrono's
        // time zone lookup, which turso_core also uses): no C is compiled for it.
        .filter(|name| *name != "core-foundation-sys")
        .filter(|name| name.ends_with("-sys") || ["cc", "openssl", "native-tls", "ring", "aws-lc-rs"].contains(name))
        .collect();
    assert!(offenders.is_empty(), "p8 depends on C: {offenders:?}");
    assert!(tree.lines().any(|l| l.starts_with("p8-interp ")), "cargo tree output not understood:\n{tree}");
}

//! The default build compiles and links no C library (SECURITY.md). This
//! fails when a dependency that builds or binds a C library joins the default
//! dependency graph, so the rule cannot erode silently.

use std::process::Command;

/// `-sys` crates allowed in the default build, and why.
const ALLOWED: &[(&str, &str)] = &[
    // Binds macOS's own CoreFoundation framework (local timezone lookup in
    // chrono); no C is compiled and nothing is linked beyond the OS.
    ("core-foundation-sys", "operating-system framework"),
    // zlib-rs's zlib-compatible API layer: Rust, despite the -sys name.
    ("libz-rs-sys", "Rust"),
];

/// C libraries still in the default build, each tracked for replacement.
/// Remove an entry when its replacement lands; never add one.
const TRACKED: &[(&str, &str)] = &[("pcre2-sys", "preg_* engine replacement")];

#[test]
fn default_build_has_no_c_libraries() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args(["tree", "-e", "normal", "--prefix", "none", "--format", "{p}"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo tree");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let tree = String::from_utf8_lossy(&out.stdout);
    let mut offenders: Vec<&str> = tree
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| {
            let c_backed = name.ends_with("-sys")
                || ["openssl", "native-tls", "ring", "aws-lc-rs", "cc"].contains(name);
            c_backed
                && !ALLOWED.iter().any(|(n, _)| n == name)
                && !TRACKED.iter().any(|(n, _)| n == name)
        })
        .collect();
    offenders.sort_unstable();
    offenders.dedup();
    assert!(
        offenders.is_empty(),
        "C-backed crates in the default build: {offenders:?} (make them opt-in features)"
    );
}

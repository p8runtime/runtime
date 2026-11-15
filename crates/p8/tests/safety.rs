//! p8's safety rules, checked so they cannot erode silently: every p8 crate
//! forbids `unsafe`, and nothing p8 depends on builds or links C.

use std::path::Path;
use std::process::Command;

fn crates_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("crates/")
}

/// The one crate allowed `unsafe` (#402): the boundary to native code.
const BOUNDARY: &str = "p8-native";

#[test]
fn every_p8_crate_forbids_unsafe() {
    let mut checked = 0;
    for entry in std::fs::read_dir(crates_dir()).expect("crates/") {
        let dir = entry.expect("entry").path();
        let name = dir.file_name().expect("name").to_string_lossy().into_owned();
        if name != "p8" && !name.starts_with("p8-") && !name.starts_with("php-") {
            continue;
        }
        if name == BOUNDARY {
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

/// The boundary crate is one file, its lints deny an `unsafe` block
/// without a reason, and each block carries its `SAFETY:` comment just
/// above it (the audit guide in its doc names what each assumes).
#[test]
fn the_boundary_is_one_audited_file() {
    let dir = crates_dir().join(BOUNDARY).join("src");
    let files: Vec<_> = std::fs::read_dir(&dir).expect("the boundary's src/").map(|e| e.expect("entry").file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(files, ["lib.rs"], "the boundary is one file");
    let source = std::fs::read_to_string(dir.join("lib.rs")).expect("lib.rs");
    assert!(source.contains("#![deny(clippy::undocumented_unsafe_blocks)]") && source.contains("#![deny(unsafe_op_in_unsafe_fn)]"));
    let lines: Vec<&str> = source.lines().collect();
    let mut blocks = 0;
    for (i, line) in lines.iter().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        if !code.contains("unsafe {") {
            continue;
        }
        blocks += 1;
        let above = lines[i.saturating_sub(4)..i].iter().any(|l| l.contains("// SAFETY"));
        assert!(above || line.contains("// SAFETY"), "line {}: an unsafe block without its SAFETY comment", i + 1);
    }
    assert!(blocks > 0, "no unsafe block found: the check reads the wrong file");
}

/// The `-sys` crates that build and link no C, each with why.
const NO_C_SYS: &[(&str, &str)] = &[
    // Rust declarations of macOS's own framework (chrono's time zone
    // lookup, which turso_core also uses): no C is compiled for it.
    ("core-foundation-sys", "macOS's framework, declared in Rust"),
    // Linux's kernel ABI as Rust constants and types (rustix's backend on
    // Linux, via turso_core): no build script, no `links`, no extern block
    // (#559). `allowed_sys_crates_build_nothing` checks it stays so.
    ("linux-raw-sys", "Linux's syscall ABI, as Rust types and constants"),
];

/// The crates that mean C is built or linked: a `-sys` crate not in
/// `NO_C_SYS`, a C compiler driver, a C library's binding.
fn c_crates(tree: &str) -> Vec<&str> {
    tree.lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| !NO_C_SYS.iter().any(|(allowed, _)| name == allowed))
        .filter(|name| name.ends_with("-sys") || ["cc", "cmake", "openssl", "native-tls", "ring", "aws-lc-rs"].contains(name))
        .collect()
}

/// `cargo tree`'s list of what p8 depends on for this platform: one
/// `name vX.Y.Z` per line.
fn p8_tree() -> String {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args(["tree", "-p", "p8", "-e", "normal", "--prefix", "none", "--format", "{p}"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo tree");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let tree = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(tree.lines().any(|l| l.starts_with("p8-interp ")), "cargo tree output not understood:\n{tree}");
    tree
}

#[test]
fn p8_links_no_c() {
    let tree = p8_tree();
    let offenders = c_crates(&tree);
    assert!(offenders.is_empty(), "p8 depends on C: {offenders:?}");
}

/// The rule itself: a C `-sys` crate or a C build tool is caught wherever
/// it is; only the reasoned exceptions pass.
#[test]
fn the_c_check_catches_c_crates() {
    let tree = "p8 v0.1.0 (/p8)\nlinux-raw-sys v0.11.0\nrustix v1.0.0\ncore-foundation-sys v0.8.7\nlibsqlite3-sys v0.30.1\ncc v1.2.0\nopenssl-sys v0.9.100\nlinux-raw-c-sys v0.1.0\n";
    // (An exception is its exact name: one like it is not let through.)
    assert_eq!(c_crates(tree), ["libsqlite3-sys", "cc", "openssl-sys", "linux-raw-c-sys"]);
    assert!(c_crates("p8 v0.1.0\nlinux-raw-sys v0.11.0\ncore-foundation-sys v0.8.7\n").is_empty());
}

/// Each `NO_C_SYS` crate in p8's tree still builds nothing: its manifest
/// (as `cargo metadata` finds it) names no build script and no native
/// library (`links`), and it has no build.rs that cargo would run.
#[test]
fn allowed_sys_crates_build_nothing() {
    let tree = p8_tree();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo metadata");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let metadata = String::from_utf8_lossy(&out.stdout);
    let manifests: Vec<&str> = metadata.split("\"manifest_path\":\"").skip(1).filter_map(|rest| rest.split('"').next()).collect();
    let mut checked = 0;
    for line in tree.lines() {
        let mut words = line.split_whitespace();
        let (Some(name), Some(version)) = (words.next(), words.next()) else { continue };
        if !NO_C_SYS.iter().any(|(allowed, _)| *allowed == name) {
            continue;
        }
        let dir = format!("/{name}-{}/Cargo.toml", version.trim_start_matches('v'));
        let manifest = manifests.iter().find(|m| m.ends_with(&dir)).unwrap_or_else(|| panic!("{name} {version}: no manifest in cargo metadata"));
        let toml = std::fs::read_to_string(manifest).expect("the crate's Cargo.toml");
        let key = |k: &str| toml.lines().map(str::trim).find(|l| l.split('=').next().is_some_and(|key| key.trim() == k)).map(str::to_owned);
        assert!(key("links").is_none(), "{name}: links a native library ({manifest})");
        let build = key("build");
        let build_rs = Path::new(manifest).with_file_name("build.rs").exists();
        assert!(
            build.as_deref().is_some_and(|b| b.ends_with("false")) || (build.is_none() && !build_rs),
            "{name}: has a build script ({manifest}): it may build C, and NO_C_SYS no longer holds for it"
        );
        checked += 1;
    }
    // (Each platform has one of them: core-foundation-sys on macOS,
    // linux-raw-sys on Linux.)
    assert!(checked > 0 || !cfg!(any(target_os = "macos", target_os = "linux")), "no NO_C_SYS crate found in p8's tree: the check reads the wrong output");
}

/// The one file that reads the process's working directory: where a
/// request starts (`p8_runtime::cwd::process_dir`, #459).
const PROCESS_DIR: &str = "p8-runtime/src/cwd.rs";

/// What reads or moves the process's working directory, which requests
/// running side by side share (#459): std's calls, and `path::absolute`,
/// which joins to it. `Command::current_dir` (a child's directory) is a
/// method, `.current_dir(`, and is not one of them.
fn reads_process_dir(line: &str) -> Option<&'static str> {
    let code = line.split("//").next().unwrap_or("");
    if code.contains("set_current_dir") {
        return Some("set_current_dir");
    }
    if code.contains("path::absolute") {
        return Some("std::path::absolute");
    }
    let mut rest = code;
    while let Some(at) = rest.find("current_dir") {
        if !rest[..at].ends_with('.') {
            return Some("current_dir");
        }
        rest = &rest[at + "current_dir".len()..];
    }
    None
}

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("a source directory") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Each request has its own working directory: no engine crate reads or
/// sets the process's but the one place a request's starts from.
#[test]
fn no_builtin_touches_the_process_working_directory() {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(crates_dir()).expect("crates/") {
        let dir = entry.expect("entry").path();
        let name = dir.file_name().expect("name").to_string_lossy().into_owned();
        if name == "p8" || name.starts_with("p8-") {
            rust_files(&dir.join("src"), &mut files);
        }
    }
    assert!(files.len() > 100, "only {} source files found", files.len());
    let mut offenders = Vec::new();
    let mut exception_seen = false;
    for file in &files {
        let shown = file.strip_prefix(crates_dir()).expect("under crates/").to_string_lossy().into_owned();
        let source = std::fs::read_to_string(file).expect("a source file");
        for (i, line) in source.lines().enumerate() {
            let Some(what) = reads_process_dir(line) else { continue };
            if shown == PROCESS_DIR && what == "current_dir" {
                exception_seen = true;
                continue;
            }
            offenders.push(format!("{shown}:{}: {what}", i + 1));
        }
    }
    assert!(exception_seen, "{PROCESS_DIR} no longer reads the process's directory: the check reads the wrong files");
    assert!(offenders.is_empty(), "the process's working directory is read or set outside {PROCESS_DIR}:\n{}", offenders.join("\n"));
}

#[test]
fn the_working_directory_check_sees_each_form() {
    assert_eq!(reads_process_dir("let d = std::env::current_dir()?;"), Some("current_dir"));
    assert_eq!(reads_process_dir("std::env::set_current_dir(&p)"), Some("set_current_dir"));
    assert_eq!(reads_process_dir("let p = std::path::absolute(x);"), Some("std::path::absolute"));
    assert_eq!(reads_process_dir("use std::env::current_dir;"), Some("current_dir"));
    assert_eq!(reads_process_dir("cmd.current_dir(cwd);"), None);
    assert_eq!(reads_process_dir("// std::env::current_dir() is the process's"), None);
}

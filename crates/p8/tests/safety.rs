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

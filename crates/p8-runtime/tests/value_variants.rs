//! The ratchet for #324: which crates still name a `Value` variant.
//!
//! `Value`'s variants become private once nothing outside
//! `p8-runtime/src/value.rs` names one (a pattern, a constructor, a
//! `matches!`): everything else goes through the accessor API
//! (`Value::kind`, `as_*`, `into_*`, `Value::int` and co). Until then this
//! test lists, per crate, the lines that still do, and fails when a crate
//! already ported names one again. A crate's port adds it to `PORTED`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Crates that name no variant any more: a regression here fails.
const PORTED: &[&str] = &["p8-runtime", "p8-interp", "p8-compiler", "p8-bytecode", "p8", "p8-ext-mbstring", "php-cli", "p8-syntax", "p8-stubs", "p8-pcre",
    "p8-ext-pdo", "p8-ext-mysqli", "p8-ext-gd", "p8-ext-pdo-pgsql", "p8-ext-pcre", "p8-ext-sockets", "p8-ext-tokenizer",
    "p8-ext-pdo-mysql", "p8-ext-pdo-sqlite", "p8-ext-fileinfo", "p8-mysqlnd",
    "p8-ext-curl", "p8-ext-openssl", "p8-ext-zip", "p8-ext-session"];

const VARIANTS: &[&str] = &["Null", "Bool", "Int", "Float", "Str", "Array", "Ref", "Object", "Undef"];

/// The one file allowed to name them.
const OWNER: &str = "p8-runtime/src/value.rs";

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "target" || n == ".target") {
                continue;
            }
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Whether `line` (comments stripped) names a `Value::<Variant>`.
fn names_variant(line: &str) -> bool {
    let code = line.split("//").next().unwrap_or("");
    let mut rest = code;
    while let Some(at) = rest.find("Value::") {
        let after = &rest[at + "Value::".len()..];
        // `ConstValue::Int` and co are other enums, not `Value`.
        let joined = rest[..at].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_');
        if joined {
            rest = after;
            continue;
        }
        let ident: String = after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if VARIANTS.contains(&ident.as_str()) {
            return true;
        }
        rest = after;
    }
    false
}

#[test]
fn only_value_rs_names_a_variant() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("crates/");
    let mut per_crate: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for crate_dir in fs::read_dir(crates).expect("crates/").flatten() {
        let name = crate_dir.file_name().to_string_lossy().into_owned();
        let mut files = Vec::new();
        rust_files(&crate_dir.path(), &mut files);
        let hits = per_crate.entry(name.clone()).or_default();
        for file in files {
            let rel = file.strip_prefix(crates).expect("under crates/").to_string_lossy().replace('\\', "/");
            if rel == OWNER {
                continue;
            }
            let Ok(text) = fs::read_to_string(&file) else { continue };
            for (i, line) in text.lines().enumerate() {
                if names_variant(line) {
                    hits.push(format!("{rel}:{}", i + 1));
                }
            }
        }
    }
    let mut report = String::from("crates still naming a Value variant (lines):\n");
    let mut regressed = Vec::new();
    for (name, hits) in &per_crate {
        if hits.is_empty() {
            continue;
        }
        report.push_str(&format!("  {name}: {}\n", hits.len()));
        if PORTED.contains(&name.as_str()) {
            regressed.push(format!("{name} is ported but names a variant at:\n    {}", hits.join("\n    ")));
        }
    }
    println!("{report}");
    assert!(regressed.is_empty(), "{}", regressed.join("\n"));
    for name in PORTED {
        assert!(per_crate.contains_key(*name), "PORTED names {name}, which is not a crate");
    }
}

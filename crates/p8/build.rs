//! The engine's build identity (`P8_ENGINE_BUILD`): a hash of the source
//! of every engine crate (all of `crates/` but the php-cli host and the
//! crates' tests), `Cargo.lock` and the target. A package of compiled files
//! (`src/package.rs`) is used only by an engine built from the same
//! source, whatever its executable is called, where it was copied to, or
//! when (a copy with a package appended to it, a container image).

use std::path::{Path, PathBuf};
use xxhash_rust::xxh3::Xxh3;

/// Every file under `dir`, but `tests` folders and hidden names.
fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "tests" {
            continue;
        }
        let path = entry.path();
        match entry.file_type() {
            Ok(t) if t.is_dir() => files(&path, out),
            Ok(t) if t.is_file() => out.push(path),
            _ => {}
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let crates = manifest.parent().expect("crates/").to_path_buf();
    let lock = crates.parent().expect("the workspace").join("Cargo.lock");
    let mut all = Vec::new();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&crates).expect("crates/").flatten().map(|e| e.path()).filter(|p| p.is_dir() && !p.ends_with("php-cli")).collect();
    dirs.sort();
    for dir in &dirs {
        println!("cargo:rerun-if-changed={}", dir.display());
        files(dir, &mut all);
    }
    println!("cargo:rerun-if-changed={}", lock.display());
    all.sort();
    all.push(lock);
    let mut h = Xxh3::new();
    h.update(std::env::var("TARGET").unwrap_or_default().as_bytes());
    for path in &all {
        let shown = path.strip_prefix(&crates).unwrap_or(path).to_string_lossy().into_owned();
        let bytes = std::fs::read(path).unwrap_or_default();
        for part in [shown.as_bytes(), &bytes] {
            h.update(&(part.len() as u64).to_le_bytes());
            h.update(part);
        }
    }
    println!("cargo:rustc-env=P8_ENGINE_BUILD={:032x}", h.digest128());
}

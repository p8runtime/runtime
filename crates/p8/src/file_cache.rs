//! Compiled files kept on disk, by content: PHP's `opcache.file_cache`.
//! A file is compiled once and read back by every later process whose
//! source, path and engine are the same.
//!
//! A cached file is `MAGIC`, the key it was written under, a SHA-256 of
//! the rest, then the program ([`p8_bytecode::serial`]). The key is a
//! SHA-256 of everything the compiled code depends on: the source, the
//! path (`__FILE__`, `__DIR__` and messages hold it), the layout's
//! version, p8's version, the running executable (any rebuild of the
//! engine: a compiler change is a new key) and the extensions (which
//! functions take references changes the code). A file that is short,
//! foreign, written under another key, or whose digest does not match is
//! not used: the file is compiled instead, and the cache entry rewritten.
//!
//! Writes go to a temporary file in the same directory, renamed into
//! place, so another process reads a whole entry or none.

use p8_bytecode::Program;
use p8_runtime::symbol::Symbols;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const MAGIC: &[u8; 8] = b"p8cache\0";
const HEADER: usize = MAGIC.len() + 32 + 32;

pub(crate) struct FileCache {
    dir: PathBuf,
    /// The key's part that is the same for every file of this Platform.
    engine: [u8; 32],
}

impl FileCache {
    /// A cache in `dir` (made if missing); None when this engine cannot
    /// tell its build apart from another's (no executable to look at),
    /// as a cache keyed only by version could serve a rebuilt engine
    /// stale code.
    pub(crate) fn new(dir: PathBuf, modules: &[&str]) -> Option<FileCache> {
        let exe = std::env::current_exe().ok()?;
        let meta = std::fs::metadata(&exe).ok()?;
        let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
        let mut h = Sha256::new();
        h.update(b"p8 file cache\0");
        h.update(p8_bytecode::serial::FORMAT.to_le_bytes());
        field(&mut h, crate::P8_VERSION.as_bytes());
        field(&mut h, exe.to_string_lossy().as_bytes());
        h.update(meta.len().to_le_bytes());
        h.update(modified.as_nanos().to_le_bytes());
        for m in modules {
            field(&mut h, m.as_bytes());
        }
        Some(FileCache { dir, engine: h.finalize().into() })
    }

    fn key(&self, source: &[u8], file: &str) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(self.engine);
        field(&mut h, file.as_bytes());
        field(&mut h, source);
        h.finalize().into()
    }

    fn path(&self, key: &[u8; 32]) -> PathBuf {
        let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
        self.dir.join(format!("{hex}.p8c"))
    }

    /// The program cached for this source and path, if there is a sound one.
    pub(crate) fn load(&self, source: &[u8], file: &str, symbols: &Symbols) -> Option<Program> {
        let key = self.key(source, file);
        let bytes = std::fs::read(self.path(&key)).ok()?;
        read(&bytes, &key, symbols)
    }

    /// Keep `program` for this source and path. Best effort: a cache that
    /// cannot be written is a cache that misses.
    pub(crate) fn store(&self, source: &[u8], file: &str, program: &Program, symbols: &Symbols) {
        let key = self.key(source, file);
        let bytes = write(program, &key, symbols);
        let _ = std::fs::create_dir_all(&self.dir);
        let target = self.path(&key);
        let temp = temp_name(&self.dir);
        if std::fs::write(&temp, &bytes).is_err() || std::fs::rename(&temp, &target).is_err() {
            let _ = std::fs::remove_file(&temp);
        }
    }
}

/// A length-prefixed field of the key (so `ab`+`c` and `a`+`bc` differ).
fn field(h: &mut Sha256, bytes: &[u8]) {
    h.update((bytes.len() as u64).to_le_bytes());
    h.update(bytes);
}

/// A cache entry's bytes for `program` under `key`.
fn write(program: &Program, key: &[u8; 32], symbols: &Symbols) -> Vec<u8> {
    let body = p8_bytecode::serial::encode(program, symbols);
    let mut out = Vec::with_capacity(HEADER + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(key);
    out.extend_from_slice(&Sha256::digest(&body));
    out.extend_from_slice(&body);
    out
}

/// The program in a cache entry, if it is one written under `key` and
/// intact.
fn read(bytes: &[u8], key: &[u8; 32], symbols: &Symbols) -> Option<Program> {
    if bytes.len() < HEADER || &bytes[..MAGIC.len()] != MAGIC || &bytes[MAGIC.len()..MAGIC.len() + 32] != key {
        return None;
    }
    let body = &bytes[HEADER..];
    if Sha256::digest(body).as_slice() != &bytes[MAGIC.len() + 32..HEADER] {
        return None;
    }
    p8_bytecode::serial::decode(body, symbols)
}

/// A name no other writer (thread or process) uses at the same time.
fn temp_name(dir: &Path) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    dir.join(format!(".{}-{n}-{nanos}.tmp", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compiled(source: &str) -> (Program, p8_runtime::ext::Registry) {
        let registry = p8_runtime::ext::Registry::new(&[crate::extensions::STANDARD]);
        let program = crate::compile(&registry, source.as_bytes(), "/x/t.php", false).expect("compiles");
        (std::sync::Arc::try_unwrap(program).expect("one owner"), registry)
    }

    const SCRIPT: &str = r#"<?php
        namespace N;
        interface I { const C = [1, 'a' => [2.5, null, true]]; }
        trait T { public function t() { return fn($x) => $x * 2; } }
        abstract class A implements I { use T { t as protected u; } abstract function f(int|string $x = self::C): ?static; }
        final class B extends A { function f(int|string $x = 1): ?static { try { return $this; } finally { echo __FILE__, __LINE__; } } }
        enum E: string { case X = 'x'; }
        function g(&$a, ...$rest) { static $n = 0; yield $n++; }
        echo (new B)->f(1) instanceof A, E::X->value, strlen("abc"), \PHP_EOL;
    "#;

    #[test]
    fn a_program_reads_back_as_written() {
        let (program, registry) = compiled(SCRIPT);
        let bytes = write(&program, &[7; 32], registry.symbols());
        // Into another symbol table: the names come back, as other numbers.
        let other = p8_runtime::ext::Registry::new(&[]);
        let back = read(&bytes, &[7; 32], other.symbols()).expect("reads back");
        assert_eq!(crate::disassemble(&back), crate::disassemble(&program));
        assert_eq!(p8_bytecode::serial::encode(&back, other.symbols()), p8_bytecode::serial::encode(&program, registry.symbols()));
        // A class declared by the file's code is the file's class, not a copy.
        assert!(std::sync::Arc::ptr_eq(&back.classes[0], &back.main.decls[0]));
    }

    #[test]
    fn a_damaged_or_foreign_entry_is_not_used() {
        let (program, registry) = compiled(SCRIPT);
        let symbols = registry.symbols();
        let bytes = write(&program, &[7; 32], symbols);
        assert!(read(&bytes, &[8; 32], symbols).is_none(), "another key's entry");
        for n in 0..bytes.len() {
            assert!(read(&bytes[..n], &[7; 32], symbols).is_none(), "cut at {n}");
            let mut flipped = bytes.clone();
            flipped[n] ^= 0x20;
            assert!(read(&flipped, &[7; 32], symbols).is_none(), "byte {n} changed");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(read(&longer, &[7; 32], symbols).is_none(), "trailing byte");
    }

    #[test]
    fn decoding_bad_bytes_never_panics() {
        // Behind the digest's back: every cut of the program itself is
        // refused, and every changed byte is refused or read, never a panic.
        let (program, registry) = compiled(SCRIPT);
        let symbols = registry.symbols();
        let body = p8_bytecode::serial::encode(&program, symbols);
        for n in 0..body.len() {
            assert!(p8_bytecode::serial::decode(&body[..n], symbols).is_none(), "cut at {n}");
            for bit in [0x01, 0x80, 0xff] {
                let mut changed = body.clone();
                changed[n] ^= bit;
                let _ = p8_bytecode::serial::decode(&changed, symbols);
            }
        }
    }
}

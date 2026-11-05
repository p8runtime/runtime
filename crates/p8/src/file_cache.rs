//! Compiled files kept on disk, by content: PHP's `opcache.file_cache`.
//! A file is compiled once and read back by every later process whose
//! source, path and engine are the same.
//!
//! An entry is `MAGIC`, the engine it was written by, a checksum of the
//! program, the file's path and source, then the program
//! ([`p8_bytecode::serial`]). It is found by a hash of the engine and the
//! path (one entry per file, as PHP's file cache has: a changed file's
//! entry is written over); it is used only when its path and source are
//! byte for byte the ones being loaded (so it can never run other code,
//! and the source needs no hashing: one comparison), its engine is this
//! one, and the program's checksum holds (so a damaged entry is never
//! read). Anything else, a short, foreign, stale or damaged file, is not
//! used: the file is compiled instead, and the entry written again.
//!
//! The engine is everything else the compiled code depends on: the
//! layout's version, p8's version, the running executable (any rebuild of
//! the engine, so a compiler change, is another engine) and the
//! extensions (which functions take references changes the code).
//!
//! Hashes are XXH3-128 (pure Rust, many GB/s): they find an entry and
//! detect damage, as PHP's file cache uses Adler-32; they do not protect
//! against whoever may write the cache directory, who is trusted as PHP
//! trusts it.
//!
//! Writes go to a temporary file in the same directory, renamed into
//! place, so another process reads a whole entry or none.

use p8_bytecode::Program;
use p8_runtime::symbol::Symbols;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use xxhash_rust::xxh3::{Xxh3, xxh3_128};

const MAGIC: &[u8; 8] = b"p8cache\0";
/// MAGIC, the engine, the checksum.
const HEADER: usize = MAGIC.len() + 16 + 16;

pub(crate) struct FileCache {
    dir: PathBuf,
    /// What compiled code depends on besides its path and source.
    engine: [u8; 16],
    /// opcache.file_cache_read_only: entries are read, never written or
    /// removed (a cache built ahead of time on a read-only disk).
    read_only: bool,
}

impl FileCache {
    /// A cache in `dir` (made if missing); None when this engine cannot
    /// tell its build apart from another's (no executable to look at),
    /// as a cache keyed only by version could serve a rebuilt engine
    /// stale code.
    pub(crate) fn new(dir: PathBuf, modules: &[&str], read_only: bool) -> Option<FileCache> {
        let exe = std::env::current_exe().ok()?;
        let meta = std::fs::metadata(&exe).ok()?;
        let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
        let mut h = Xxh3::new();
        h.update(b"p8 file cache\0");
        h.update(&p8_bytecode::serial::FORMAT.to_le_bytes());
        field(&mut h, crate::P8_VERSION.as_bytes());
        field(&mut h, exe.to_string_lossy().as_bytes());
        h.update(&meta.len().to_le_bytes());
        h.update(&modified.as_nanos().to_le_bytes());
        for m in modules {
            field(&mut h, m.as_bytes());
        }
        Some(FileCache { dir, engine: h.digest128().to_le_bytes(), read_only })
    }

    /// Where the entry for this path is.
    fn path(&self, file: &str) -> PathBuf {
        let mut h = Xxh3::new();
        h.update(&self.engine);
        field(&mut h, file.as_bytes());
        self.dir.join(format!("{:032x}.p8c", h.digest128()))
    }

    /// The program cached for this source and path, if there is a sound one.
    pub(crate) fn load(&self, source: &[u8], file: &str, symbols: &Symbols) -> Option<Program> {
        let bytes = std::fs::read(self.path(file)).ok()?;
        read(&bytes, &self.engine, file, source, symbols)
    }

    /// Keep `program` for this source and path. Best effort: a cache that
    /// cannot be written is a cache that misses.
    pub(crate) fn store(&self, source: &[u8], file: &str, program: &Program, symbols: &Symbols) {
        use std::io::Write;
        if self.read_only {
            return;
        }
        let (head, table, body) = parts(program, &self.engine, file, source, symbols);
        let _ = std::fs::create_dir_all(&self.dir);
        let target = self.path(file);
        let temp = temp_name(&self.dir);
        // Written piece by piece: the source and program are not copied
        // into one buffer first.
        let written = std::fs::File::create(&temp).and_then(|mut f| {
            for piece in [&head[..], source, &table, &body] {
                f.write_all(piece)?;
            }
            Ok(())
        });
        if written.is_err() || std::fs::rename(&temp, &target).is_err() {
            let _ = std::fs::remove_file(&temp);
        }
    }

    /// Drop the entry for this path (zend_file_cache_invalidate).
    pub(crate) fn forget(&self, file: &str) {
        if self.read_only {
            return;
        }
        let _ = std::fs::remove_file(self.path(file));
    }
}

/// A length-prefixed field of a hash (so `ab`+`c` and `a`+`bc` differ).
fn field(h: &mut Xxh3, bytes: &[u8]) {
    h.update(&(bytes.len() as u64).to_le_bytes());
    h.update(bytes);
}

/// An entry for `program`, compiled by `engine` from `source` at `file`,
/// in the pieces it is written in: what comes before the source (MAGIC,
/// the engine, the program's checksum, the path, the source's length),
/// then (after the source) the program's string table and the program.
fn parts(program: &Program, engine: &[u8; 16], file: &str, source: &[u8], symbols: &Symbols) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let (table, body) = p8_bytecode::serial::encode_parts(program, symbols);
    let mut sum = Xxh3::new();
    sum.update(&table);
    sum.update(&body);
    let mut head = Vec::with_capacity(HEADER + 16 + file.len());
    head.extend_from_slice(MAGIC);
    head.extend_from_slice(engine);
    head.extend_from_slice(&sum.digest128().to_le_bytes());
    head.extend_from_slice(&(file.len() as u64).to_le_bytes());
    head.extend_from_slice(file.as_bytes());
    head.extend_from_slice(&(source.len() as u64).to_le_bytes());
    (head, table, body)
}

/// An entry's bytes, joined (for tests).
#[cfg(test)]
fn write(program: &Program, engine: &[u8; 16], file: &str, source: &[u8], symbols: &Symbols) -> Vec<u8> {
    let (head, table, body) = parts(program, engine, file, source, symbols);
    [&head[..], source, &table, &body].concat()
}

/// The program in an entry, if `engine` wrote it, from exactly this
/// `file` and `source`, and it is intact. The path and source are
/// compared byte for byte (which also proves them intact); the checksum
/// covers the program.
fn read(bytes: &[u8], engine: &[u8; 16], file: &str, source: &[u8], symbols: &Symbols) -> Option<Program> {
    let header = bytes.get(..HEADER)?;
    if &header[..MAGIC.len()] != MAGIC || &header[MAGIC.len()..MAGIC.len() + 16] != engine {
        return None;
    }
    let mut rest = &bytes[HEADER..];
    for expected in [file.as_bytes(), source] {
        let len = usize::try_from(u64::from_le_bytes(rest.get(..8)?.try_into().ok()?)).ok()?;
        let part = rest.get(8..8usize.checked_add(len)?)?;
        if part != expected {
            return None;
        }
        rest = &rest[8 + len..];
    }
    if xxh3_128(rest).to_le_bytes() != header[MAGIC.len() + 16..] {
        return None;
    }
    p8_bytecode::serial::decode(rest, symbols)
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
        let bytes = write(&program, &[7; 16], "/x/t.php", SCRIPT.as_bytes(), registry.symbols());
        // Into another symbol table: the names come back, as other numbers.
        let other = p8_runtime::ext::Registry::new(&[]);
        let back = read(&bytes, &[7; 16], "/x/t.php", SCRIPT.as_bytes(), other.symbols()).expect("reads back");
        assert_eq!(crate::disassemble(&back), crate::disassemble(&program));
        assert_eq!(p8_bytecode::serial::encode(&back, other.symbols()), p8_bytecode::serial::encode(&program, registry.symbols()));
        // A class declared by the file's code is the file's class, not a copy.
        assert!(std::sync::Arc::ptr_eq(&back.classes[0], &back.main.decls[0]));
    }

    #[test]
    fn a_damaged_or_foreign_entry_is_not_used() {
        let (program, registry) = compiled(SCRIPT);
        let symbols = registry.symbols();
        let source = SCRIPT.as_bytes();
        let bytes = write(&program, &[7; 16], "/x/t.php", source, symbols);
        let read = |bytes: &[u8]| read(bytes, &[7; 16], "/x/t.php", source, symbols);
        assert!(read(&bytes).is_some());
        assert!(super::read(&bytes, &[8; 16], "/x/t.php", source, symbols).is_none(), "another engine's entry");
        assert!(super::read(&bytes, &[7; 16], "/x/u.php", source, symbols).is_none(), "another file's entry");
        assert!(super::read(&bytes, &[7; 16], "/x/t.php", &source[1..], symbols).is_none(), "another source's entry");
        for n in 0..bytes.len() {
            assert!(read(&bytes[..n]).is_none(), "cut at {n}");
            let mut flipped = bytes.clone();
            flipped[n] ^= 0x20;
            assert!(read(&flipped).is_none(), "byte {n} changed");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(read(&longer).is_none(), "trailing byte");
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

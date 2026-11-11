//! A package: every compiled file of a project in one file, written ahead
//! of time (`php2 --compile <dir>`), so that a run takes each file it
//! includes from the package and parses and compiles none of them (#381).
//!
//! A packaged file is used only while it is the file on disk: the
//! package holds each file's path, size, modification time and source
//! hash (the manifest of #382), and a run checks a file against it before
//! taking its code ([`PackageCheck`]; by default the file's source is
//! read and hashed). A file that changed, a file the package does not
//! have (a plugin installed later) and `eval()`'d code are compiled from
//! their source as without a package (#383). A package that is damaged,
//! cut short, or written by another build of the engine is not used.
//!
//! Layout, little-endian:
//!
//! - header (`HEADER` bytes): `MAGIC`; the engine (16 bytes, see
//!   [`engine`]); the number of files (u32); 0 (u32); the package's length
//!   (u64); the index's length (u64); a checksum (XXH3-128) of the header
//!   before it and of the index.
//! - index: one `RECORD` per file, sorted by path: where its path is in
//!   the path list (u32 offset, u32 length), its source's length (u64),
//!   modification time (i64 seconds, u32 nanoseconds), 0 (u32), source hash
//!   (XXH3-128), where its program is (u64 offset from the package's start,
//!   u64 length) and the program's checksum (XXH3-128); then the paths,
//!   back to back.
//! - the programs: each file's as the file cache stores it
//!   ([`p8_bytecode::serial`]).
//!
//! Opening a package reads its header and index only; a file's program
//! is read, checked and decoded when the file is first included, so what
//! a run never includes costs it nothing. The bytes come from a file:
//! a package on its own (`.p8/package` beside the source), or the end of
//! an executable it was appended to (`TAIL`), or from memory.
//!
//! The hashes find stale files and damaged bytes; they do not protect
//! against whoever may write the package, who is trusted as the source
//! folder is.

use p8_bytecode::Program;
use p8_runtime::symbol::Symbols;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use xxhash_rust::xxh3::{Xxh3, xxh3_128};

const MAGIC: &[u8; 8] = b"p8pack\0\0";
/// The last bytes of an executable with a package appended: the
/// package's length (u64), then these.
const TAIL: &[u8; 8] = b"p8pkgend";
/// MAGIC, the engine, the count and a zero, the length, the index's
/// length, the checksum.
const HEADER: usize = 8 + 16 + 8 + 8 + 8 + 16;
/// What the checksum covers of the header: all before it.
const SUMMED: usize = HEADER - 16;
const RECORD: usize = 80;

/// How a run checks a packaged file against the file on disk before it
/// takes the file's code from the package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PackageCheck {
    /// The file's source is read and hashed: its code is taken from the
    /// package only when the hash is the one packaged (#382).
    #[default]
    Hash,
    /// The file's size and modification time: when both are the
    /// packaged ones its source is not read; when either differs it is
    /// read and hashed, as with `Hash`. A file changed without either
    /// changing (written within the clock's resolution, to the same
    /// size, or given its old time back) runs as packaged.
    Stat,
    /// The package is the code of every file in it: no file is looked
    /// at (as PHP's `opcache.validate_timestamps=0`). A changed file runs
    /// as packaged until the package is written again.
    Trust,
}

/// Why a package is not used.
#[derive(Debug)]
pub enum PackageError {
    Io(std::io::Error),
    /// Not a package (or an executable with none appended).
    NotAPackage,
    /// Cut short, extended, or a byte changed.
    Damaged,
    /// Written by another build of the engine.
    OtherEngine,
}

impl std::fmt::Display for PackageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackageError::Io(e) => write!(f, "{e}"),
            PackageError::NotAPackage => f.write_str("not a p8 package"),
            PackageError::Damaged => f.write_str("the package is damaged"),
            PackageError::OtherEngine => f.write_str("the package was written by another build of p8"),
        }
    }
}

/// What compiled code depends on besides its path and source: the
/// layout's version, p8's version, the engine's build (a hash of its
/// source, see build.rs: any change to the compiler is another engine)
/// and the extensions (which functions take references changes the
/// code). Unlike the file cache's key it does not name the executable, so
/// a copy of it (a container image, a copy with the package appended)
/// uses the package.
pub(crate) fn engine(modules: &[&str]) -> [u8; 16] {
    let mut h = Xxh3::new();
    h.update(b"p8 package\0");
    h.update(&p8_bytecode::serial::FORMAT.to_le_bytes());
    for part in [crate::P8_VERSION, env!("P8_ENGINE_BUILD")].into_iter().chain(modules.iter().copied()) {
        h.update(&(part.len() as u64).to_le_bytes());
        h.update(part.as_bytes());
    }
    h.digest128().to_le_bytes()
}

/// Where a package's bytes are read from.
enum Bytes {
    /// A file, the package starting at `base` (an executable it was
    /// appended to).
    File { file: Mutex<std::fs::File>, base: u64 },
    Memory(Vec<u8>),
}

impl Bytes {
    /// `len` bytes at `at` (from the package's start), if there are.
    fn read(&self, at: u64, len: usize) -> Option<Vec<u8>> {
        match self {
            Bytes::File { file, base } => {
                let mut file = file.lock().unwrap_or_else(|e| e.into_inner());
                file.seek(SeekFrom::Start(base.checked_add(at)?)).ok()?;
                let mut bytes = vec![0; len];
                file.read_exact(&mut bytes).ok()?;
                Some(bytes)
            }
            Bytes::Memory(all) => {
                let at = usize::try_from(at).ok()?;
                all.get(at..at.checked_add(len)?).map(<[u8]>::to_vec)
            }
        }
    }
}

/// A file as the index has it.
struct Record<'a> {
    path: &'a [u8],
    size: u64,
    modified: (i64, u32),
    hash: &'a [u8],
    at: u64,
    len: u64,
    sum: &'a [u8],
}

/// An opened package: its index, and where to read programs from.
pub struct Package {
    bytes: Bytes,
    count: usize,
    /// The records, then the paths.
    index: Vec<u8>,
    check: PackageCheck,
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// A file's modification time, as the index keeps it.
fn modified(meta: &std::fs::Metadata) -> Option<(i64, u32)> {
    let t = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some((i64::try_from(t.as_secs()).ok()?, t.subsec_nanos()))
}

impl Package {
    /// The package in the file at `path`, if it is one this engine wrote.
    pub(crate) fn open(path: &Path, engine: &[u8; 16], check: PackageCheck) -> Result<Package, PackageError> {
        let file = std::fs::File::open(path).map_err(PackageError::Io)?;
        let len = file.metadata().map_err(PackageError::Io)?.len();
        Package::from(Bytes::File { file: Mutex::new(file), base: 0 }, len, engine, check)
    }

    /// The package appended to the executable at `path`: Ok(None) when it
    /// has none.
    pub(crate) fn appended(path: &Path, engine: &[u8; 16], check: PackageCheck) -> Result<Option<Package>, PackageError> {
        let mut file = std::fs::File::open(path).map_err(PackageError::Io)?;
        let len = file.metadata().map_err(PackageError::Io)?.len();
        let Some(tail_at) = len.checked_sub(16) else { return Ok(None) };
        let mut tail = [0; 16];
        file.seek(SeekFrom::Start(tail_at)).and_then(|_| file.read_exact(&mut tail)).map_err(PackageError::Io)?;
        if &tail[8..] != TAIL {
            return Ok(None);
        }
        let size = u64::from_le_bytes(tail[..8].try_into().expect("8 bytes"));
        let base = tail_at.checked_sub(size).ok_or(PackageError::Damaged)?;
        Package::from(Bytes::File { file: Mutex::new(file), base }, size, engine, check).map(Some)
    }

    /// The package `bytes` hold.
    pub(crate) fn in_memory(bytes: Vec<u8>, engine: &[u8; 16], check: PackageCheck) -> Result<Package, PackageError> {
        let len = bytes.len() as u64;
        Package::from(Bytes::Memory(bytes), len, engine, check)
    }

    fn from(bytes: Bytes, len: u64, engine: &[u8; 16], check: PackageCheck) -> Result<Package, PackageError> {
        let header = bytes.read(0, HEADER).ok_or(PackageError::NotAPackage)?;
        if &header[..8] != MAGIC {
            return Err(PackageError::NotAPackage);
        }
        let count = u32_at(&header, 24).ok_or(PackageError::Damaged)? as usize;
        let total = u64_at(&header, 32).ok_or(PackageError::Damaged)?;
        let index_len = u64_at(&header, 40).ok_or(PackageError::Damaged)?;
        if total != len || index_len > len.saturating_sub(HEADER as u64) || (index_len as usize) < count.checked_mul(RECORD).ok_or(PackageError::Damaged)? {
            return Err(PackageError::Damaged);
        }
        let index = bytes.read(HEADER as u64, index_len as usize).ok_or(PackageError::Damaged)?;
        let mut sum = Xxh3::new();
        sum.update(&header[..SUMMED]);
        sum.update(&index);
        if sum.digest128().to_le_bytes() != header[SUMMED..] {
            return Err(PackageError::Damaged);
        }
        if &header[8..24] != engine {
            return Err(PackageError::OtherEngine);
        }
        Ok(Package { bytes, count, index, check })
    }

    /// How many files it holds.
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub(crate) fn check(&self) -> PackageCheck {
        self.check
    }

    fn record(&self, i: usize) -> Option<Record<'_>> {
        let at = i.checked_mul(RECORD)?;
        let r = self.index.get(at..at + RECORD)?;
        let paths = self.index.get(self.count * RECORD..)?;
        let (from, len) = (u32_at(r, 0)? as usize, u32_at(r, 4)? as usize);
        Some(Record {
            path: paths.get(from..from.checked_add(len)?)?,
            size: u64_at(r, 8)?,
            modified: (u64_at(r, 16)? as i64, u32_at(r, 24)?),
            hash: &r[32..48],
            at: u64_at(r, 48)?,
            len: u64_at(r, 56)?,
            sum: &r[64..80],
        })
    }

    /// The record of the file at `file` (a resolved path).
    fn find(&self, file: &str) -> Option<Record<'_>> {
        let (mut lo, mut hi) = (0, self.count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let r = self.record(mid)?;
            match r.path.cmp(file.as_bytes()) {
                std::cmp::Ordering::Equal => return Some(r),
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
            }
        }
        None
    }

    /// A file's program, read and checked.
    fn program(&self, r: &Record, symbols: &Symbols) -> Option<Program> {
        let bytes = self.bytes.read(r.at, usize::try_from(r.len).ok()?)?;
        if xxh3_128(&bytes).to_le_bytes() != r.sum {
            return None;
        }
        p8_bytecode::serial::decode(&bytes, symbols)
    }

    /// The program packaged for `file`, and its source's hash, when
    /// `source` is the source it was compiled from (and the package's
    /// bytes for it are sound); else None, and the file is compiled.
    pub(crate) fn load(&self, source: &[u8], file: &str, symbols: &Symbols) -> Option<(Program, [u8; 16])> {
        let r = self.find(file)?;
        let hash = xxh3_128(source).to_le_bytes();
        if r.size != source.len() as u64 || hash != r.hash {
            return None;
        }
        Some((self.program(&r, symbols)?, hash))
    }

    /// Whether the package vouches for `file` without its source being
    /// read: with `Stat`, while its size and modification time are the
    /// packaged ones; with `Trust`, whenever the package has it. Else
    /// the source is to be read (and [`Package::load`] asked).
    pub(crate) fn vouches(&self, file: &str) -> bool {
        let Some(r) = self.find(file) else { return false };
        match self.check {
            PackageCheck::Hash => false,
            PackageCheck::Stat => std::fs::metadata(file).ok().is_some_and(|meta| meta.len() == r.size && modified(&meta) == Some(r.modified)),
            PackageCheck::Trust => true,
        }
    }

    /// The program packaged for `file`, and its source's length, unchecked
    /// against the file (for one [`Package::vouches`] for).
    pub(crate) fn unread(&self, file: &str, symbols: &Symbols) -> Option<(Program, u64)> {
        let r = self.find(file)?;
        Some((self.program(&r, symbols)?, r.size))
    }
}

/// A file compiled for a package.
pub(crate) struct Built {
    pub path: String,
    pub size: u64,
    pub modified: (i64, u32),
    pub hash: [u8; 16],
    /// Its program (the file cache's bytes).
    pub program: Vec<u8>,
}

impl Built {
    pub(crate) fn new(path: String, source: &[u8], meta: &std::fs::Metadata, program: Vec<u8>) -> Built {
        Built { path, size: source.len() as u64, modified: modified(meta).unwrap_or((0, 0)), hash: xxh3_128(source).to_le_bytes(), program }
    }
}

/// A package of `files` (by `engine`), as bytes.
pub(crate) fn bytes(mut files: Vec<Built>, engine: &[u8; 16]) -> Vec<u8> {
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files.dedup_by(|a, b| a.path == b.path);
    let paths: usize = files.iter().map(|f| f.path.len()).sum();
    let index_len = files.len() * RECORD + paths;
    let programs: usize = files.iter().map(|f| f.program.len()).sum();
    let total = HEADER + index_len + programs;
    let mut index = Vec::with_capacity(index_len);
    let (mut path_at, mut program_at) = (0u32, (HEADER + index_len) as u64);
    for f in &files {
        index.extend_from_slice(&path_at.to_le_bytes());
        index.extend_from_slice(&(f.path.len() as u32).to_le_bytes());
        index.extend_from_slice(&f.size.to_le_bytes());
        index.extend_from_slice(&f.modified.0.to_le_bytes());
        index.extend_from_slice(&f.modified.1.to_le_bytes());
        index.extend_from_slice(&0u32.to_le_bytes());
        index.extend_from_slice(&f.hash);
        index.extend_from_slice(&program_at.to_le_bytes());
        index.extend_from_slice(&(f.program.len() as u64).to_le_bytes());
        index.extend_from_slice(&xxh3_128(&f.program).to_le_bytes());
        path_at += f.path.len() as u32;
        program_at += f.program.len() as u64;
    }
    for f in &files {
        index.extend_from_slice(f.path.as_bytes());
    }
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(engine);
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(total as u64).to_le_bytes());
    out.extend_from_slice(&(index_len as u64).to_le_bytes());
    let mut sum = Xxh3::new();
    sum.update(&out);
    sum.update(&index);
    out.extend_from_slice(&sum.digest128().to_le_bytes());
    out.extend_from_slice(&index);
    for f in &files {
        out.extend_from_slice(&f.program);
    }
    out
}

/// Write `pieces` to `target` whole or not at all: into a temporary file
/// beside it, renamed into place (a run reading the old package keeps
/// reading the old one). `like` gives the new file its permissions.
pub(crate) fn write_file(target: &Path, pieces: &[&[u8]], like: Option<&Path>) -> std::io::Result<()> {
    let dir = target.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = target.file_name().map_or_else(|| "package".into(), |n| n.to_string_lossy().into_owned());
    let temp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let written = (|| {
        let mut f = std::fs::File::create(&temp)?;
        for piece in pieces {
            f.write_all(piece)?;
        }
        f.sync_all()?;
        if let Some(like) = like {
            std::fs::set_permissions(&temp, std::fs::metadata(like)?.permissions())?;
        }
        std::fs::rename(&temp, target)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}

/// What ends an executable that `package` is appended to.
pub(crate) fn tail(package: &[u8]) -> Vec<u8> {
    [&(package.len() as u64).to_le_bytes()[..], TAIL].concat()
}

/// How much of `exe` is the executable: all of it, or what comes before
/// the package appended to it.
pub(crate) fn executable_len(exe: &[u8]) -> usize {
    let Some(tail_at) = exe.len().checked_sub(16) else { return exe.len() };
    if &exe[tail_at + 8..] != TAIL {
        return exe.len();
    }
    let size = u64::from_le_bytes(exe[tail_at..tail_at + 8].try_into().expect("8 bytes"));
    usize::try_from(size).ok().and_then(|size| tail_at.checked_sub(size)).unwrap_or(exe.len())
}

/// The PHP files under `root`, sorted: `.php`, `.inc` and `.phtml` files,
/// not in hidden folders (`.p8`, `.git`); a link to a folder is not
/// followed (it may lead back up).
pub(crate) fn sources(root: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => walk(&path, out),
                Ok(_) if path.is_file() && path.extension().is_some_and(|e| e == "php" || e == "inc" || e == "phtml") => out.push(path),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(path: &str, source: &str, registry: &p8_runtime::ext::Registry) -> Built {
        let program = crate::compile(registry, source.as_bytes(), path, false).expect("compiles");
        let bytes = p8_bytecode::serial::encode(&program, registry.symbols());
        Built { path: path.into(), size: source.len() as u64, modified: (1, 2), hash: xxh3_128(source.as_bytes()).to_le_bytes(), program: bytes }
    }

    fn sample(engine: &[u8; 16]) -> (Vec<u8>, p8_runtime::ext::Registry) {
        let registry = p8_runtime::ext::Registry::new(&[crate::extensions::STANDARD]);
        let files = vec![built("/p/b.php", "<?php function b() { return 2; }", &registry), built("/p/a.php", "<?php echo 1;", &registry)];
        (bytes(files, engine), registry)
    }

    #[test]
    fn a_package_finds_its_files_by_path_and_source() {
        let (bytes, registry) = sample(&[7; 16]);
        let symbols = registry.symbols();
        let p = Package::in_memory(bytes, &[7; 16], PackageCheck::Hash).ok().expect("opens");
        assert_eq!(p.len(), 2);
        assert!(p.load(b"<?php echo 1;", "/p/a.php", symbols).is_some());
        assert!(p.load(b"<?php function b() { return 2; }", "/p/b.php", symbols).is_some());
        assert!(p.load(b"<?php echo 2;", "/p/a.php", symbols).is_none(), "another source");
        assert!(p.load(b"<?php echo 1;", "/p/c.php", symbols).is_none(), "a file it does not have");
        assert!(!p.vouches("/p/a.php"), "Hash reads every source");
        let trusting = Package::in_memory(sample(&[7; 16]).0, &[7; 16], PackageCheck::Trust).ok().expect("opens");
        assert!(trusting.vouches("/p/a.php") && trusting.unread("/p/a.php", symbols).is_some());
        assert!(!trusting.vouches("/p/c.php") && trusting.unread("/p/c.php", symbols).is_none());
        // Stat: the packaged time (1 s 2 ns) is not that of a file on disk.
        let stat = Package::in_memory(sample(&[7; 16]).0, &[7; 16], PackageCheck::Stat).ok().expect("opens");
        assert!(!stat.vouches("/p/a.php"));
    }

    #[test]
    fn another_engines_package_is_not_used() {
        let (bytes, _) = sample(&[7; 16]);
        assert!(matches!(Package::in_memory(bytes, &[8; 16], PackageCheck::Hash), Err(PackageError::OtherEngine)));
    }

    #[test]
    fn a_damaged_package_or_entry_is_not_used() {
        let (bytes, registry) = sample(&[7; 16]);
        let symbols = registry.symbols();
        let open = |b: Vec<u8>| Package::in_memory(b, &[7; 16], PackageCheck::Hash);
        let index_end = HEADER + u64_at(&bytes, 40).expect("index length") as usize;
        for n in 0..bytes.len() {
            assert!(open(bytes[..n].to_vec()).is_err(), "cut at {n}");
            let mut changed = bytes.clone();
            changed[n] ^= 0x20;
            match open(changed) {
                // A byte of the header or the index: the package is refused.
                Err(_) => assert!(n < index_end, "byte {n} of a program refused the package"),
                // A byte of a program: that file is not taken from it.
                Ok(p) => {
                    assert!(n >= index_end, "byte {n} of the index went unnoticed");
                    let a = p.load(b"<?php echo 1;", "/p/a.php", symbols).is_some();
                    let b = p.load(b"<?php function b() { return 2; }", "/p/b.php", symbols).is_some();
                    assert!(!(a && b), "byte {n} changed and both files were used");
                }
            }
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(matches!(open(longer), Err(PackageError::Damaged)), "a trailing byte");
        assert!(matches!(open(b"not a package at all, but long enough to have a header: 0123456789".to_vec()), Err(PackageError::NotAPackage)));
    }
}

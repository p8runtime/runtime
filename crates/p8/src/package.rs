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
//!   (u64); where the index starts (u64; it runs to the end); a checksum
//!   (XXH3-128) of the header before it and of the index.
//! - the programs: each file's as the file cache stores it
//!   ([`p8_bytecode::serial`]), written as they are compiled.
//! - index: one `RECORD` per file, sorted by path: where its path is in
//!   the path list (u32 offset, u32 length), its source's length (u64),
//!   modification time (i64 seconds, u32 nanoseconds), 0 (u32), source hash
//!   (XXH3-128), where its program is (u64 offset from the package's start,
//!   u64 length) and the program's checksum (XXH3-128); then the paths,
//!   back to back.
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
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use xxhash_rust::xxh3::{Xxh3, xxh3_128};

const MAGIC: &[u8; 8] = b"p8pack\0\0";
/// The last bytes of an executable with a package appended: the
/// package's length (u64), then these.
const TAIL: &[u8; 8] = b"p8pkgend";
/// MAGIC, the engine, the count and a zero, the length, where the index
/// starts, the checksum.
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
                // Read into the spare capacity: the buffer is not zeroed first.
                let mut bytes = Vec::with_capacity(len);
                (&mut *file).take(len as u64).read_to_end(&mut bytes).ok()?;
                (bytes.len() == len).then_some(bytes)
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
    /// Its number in the index (the order of the paths).
    number: u32,
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
    /// The native functions of the executable carrying the package, by
    /// file (its record's number): each (function's index in the file's
    /// `Program::functions`, native id). Empty without native code.
    native: HashMap<u32, Vec<(u32, u32)>>,
}

/// A function of a package, as the native backend compiles it: which
/// file (its record's number) and which of the file's functions.
pub struct PackagedFunction {
    pub file: u32,
    pub index: u32,
    pub function: Arc<p8_bytecode::Function>,
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// A file's modification time, as the index keeps it.
pub(crate) fn modified(meta: &std::fs::Metadata) -> Option<(i64, u32)> {
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
        let index_at = u64_at(&header, 40).ok_or(PackageError::Damaged)?;
        let index_len = usize::try_from(len.checked_sub(index_at).ok_or(PackageError::Damaged)?).map_err(|_| PackageError::Damaged)?;
        if total != len || index_at < HEADER as u64 || index_len < count.checked_mul(RECORD).ok_or(PackageError::Damaged)? {
            return Err(PackageError::Damaged);
        }
        let index = bytes.read(index_at, index_len).ok_or(PackageError::Damaged)?;
        let mut sum = Xxh3::new();
        sum.update(&header[..SUMMED]);
        sum.update(&index);
        if sum.digest128().to_le_bytes() != header[SUMMED..] {
            return Err(PackageError::Damaged);
        }
        if &header[8..24] != engine {
            return Err(PackageError::OtherEngine);
        }
        Ok(Package { bytes, count, index, check, native: HashMap::new() })
    }

    /// Give the functions `keys` names (by id: file, function) their
    /// native ids as each file is read (`program`).
    pub(crate) fn with_native(mut self, keys: &[(u32, u32)]) -> Package {
        self.native.clear();
        for (id, &(file, index)) in keys.iter().enumerate() {
            self.native.entry(file).or_default().push((index, id as u32));
        }
        self
    }

    /// Every function of every file, read and checked (a file whose
    /// program is damaged has none).
    pub(crate) fn functions(&self, symbols: &Symbols) -> Vec<PackagedFunction> {
        let mut out = Vec::new();
        for i in 0..self.count {
            let Some(program) = self.record(i).and_then(|r| self.program(&r, symbols)) else { continue };
            for (index, function) in program.functions.into_iter().enumerate() {
                out.push(PackagedFunction { file: i as u32, index: index as u32, function });
            }
        }
        out
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
            number: u32::try_from(i).ok()?,
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
        let mut program = p8_bytecode::serial::decode(&bytes, symbols)?;
        // Native ids, given before the code is shared (APS 5: none of it
        // changes after); a function already shared keeps none.
        for &(index, id) in self.native.get(&r.number).map_or(&[][..], Vec::as_slice) {
            if let Some(f) = program.functions.get_mut(index as usize).and_then(Arc::get_mut) {
                f.native = Some(id);
            }
        }
        Some(program)
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

/// A packaged file, as the writer keeps it until the index is written.
struct Entry {
    path: String,
    size: u64,
    modified: (i64, u32),
    hash: [u8; 16],
    at: u64,
    len: u64,
    sum: [u8; 16],
}

/// Writes a package: each program as it is compiled (so a project's
/// programs are never all in memory at once), then the index, then the
/// header.
pub(crate) struct Writer<W: Write + Seek> {
    out: W,
    at: u64,
    entries: Vec<Entry>,
}

impl<W: Write + Seek> Writer<W> {
    pub(crate) fn new(mut out: W) -> std::io::Result<Writer<W>> {
        out.write_all(&[0; HEADER])?;
        Ok(Writer { out, at: HEADER as u64, entries: Vec::new() })
    }

    /// The file at `path` (resolved), modified at `modified` (see
    /// [`modified`]), compiled from `source` to `program` (its serial
    /// bytes).
    pub(crate) fn add(&mut self, path: String, modified: (i64, u32), source: &[u8], program: &[u8]) -> std::io::Result<()> {
        self.out.write_all(program)?;
        let len = program.len() as u64;
        let entry = Entry {
            path,
            size: source.len() as u64,
            modified,
            hash: xxh3_128(source).to_le_bytes(),
            at: self.at,
            len,
            sum: xxh3_128(program).to_le_bytes(),
        };
        self.entries.push(entry);
        self.at += len;
        Ok(())
    }

    /// The index and the header written for `engine`; how many files
    /// and bytes the package has.
    pub(crate) fn finish(mut self, engine: &[u8; 16]) -> std::io::Result<(W, usize, u64)> {
        let mut entries = std::mem::take(&mut self.entries);
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        entries.dedup_by(|a, b| a.path == b.path);
        let paths: usize = entries.iter().map(|e| e.path.len()).sum();
        let mut index = Vec::with_capacity(entries.len() * RECORD + paths);
        let too_long = || std::io::Error::other("a package's paths are under 4 GiB");
        let mut path_at = 0u32;
        for e in &entries {
            let len = u32::try_from(e.path.len()).map_err(|_| too_long())?;
            index.extend_from_slice(&path_at.to_le_bytes());
            index.extend_from_slice(&len.to_le_bytes());
            index.extend_from_slice(&e.size.to_le_bytes());
            index.extend_from_slice(&e.modified.0.to_le_bytes());
            index.extend_from_slice(&e.modified.1.to_le_bytes());
            index.extend_from_slice(&0u32.to_le_bytes());
            index.extend_from_slice(&e.hash);
            index.extend_from_slice(&e.at.to_le_bytes());
            index.extend_from_slice(&e.len.to_le_bytes());
            index.extend_from_slice(&e.sum);
            path_at = path_at.checked_add(len).ok_or_else(too_long)?;
        }
        for e in &entries {
            index.extend_from_slice(e.path.as_bytes());
        }
        let count = u32::try_from(entries.len()).map_err(|_| std::io::Error::other("a package holds under 2^32 files"))?;
        let total = self.at + index.len() as u64;
        let mut header = Vec::with_capacity(HEADER);
        header.extend_from_slice(MAGIC);
        header.extend_from_slice(engine);
        header.extend_from_slice(&count.to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&total.to_le_bytes());
        header.extend_from_slice(&self.at.to_le_bytes());
        let mut sum = Xxh3::new();
        sum.update(&header);
        sum.update(&index);
        header.extend_from_slice(&sum.digest128().to_le_bytes());
        self.out.write_all(&index)?;
        self.out.seek(SeekFrom::Start(0))?;
        self.out.write_all(&header)?;
        Ok((self.out, entries.len(), total))
    }
}

/// Write `target` whole or not at all: `write` fills a temporary file
/// beside it, which is then renamed into place (a run reading the old
/// file keeps reading the old one). `like` gives it its permissions.
pub(crate) fn replace<T>(target: &Path, like: Option<&Path>, write: impl FnOnce(std::fs::File) -> std::io::Result<(std::fs::File, T)>) -> std::io::Result<T> {
    let dir = target.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = target.file_name().map_or_else(|| "package".into(), |n| n.to_string_lossy().into_owned());
    let temp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let written = (|| {
        let (file, made) = write(std::fs::File::create(&temp)?)?;
        file.sync_all()?;
        if let Some(like) = like {
            std::fs::set_permissions(&temp, std::fs::metadata(like)?.permissions())?;
        }
        std::fs::rename(&temp, target)?;
        Ok(made)
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

    /// A package of /p/a.php and /p/b.php (added out of order), by `engine`.
    fn sample(engine: &[u8; 16]) -> (Vec<u8>, p8_runtime::ext::Registry) {
        let registry = p8_runtime::ext::Registry::new(&[crate::extensions::STANDARD]);
        let mut w = Writer::new(std::io::Cursor::new(Vec::new())).expect("writer");
        for (path, source) in [("/p/b.php", "<?php function b() { return 2; }"), ("/p/a.php", "<?php echo 1;")] {
            let program = crate::compile(&registry, source.as_bytes(), path, false).expect("compiles");
            w.add(path.into(), (1, 2), source.as_bytes(), &p8_bytecode::serial::encode(&program, registry.symbols())).expect("add");
        }
        let (out, count, total) = w.finish(engine).expect("finish");
        let bytes = out.into_inner();
        assert_eq!((count, total), (2, bytes.len() as u64));
        (bytes, registry)
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
    fn the_engine_is_its_build_and_its_extensions() {
        // build.rs: a hash of the engine's source (ops.def, the compiler,
        // the serial layout...), so any rebuild from changed source, such
        // as one adding opcodes, is another engine.
        let build = env!("P8_ENGINE_BUILD");
        assert!(build.len() == 32 && build.bytes().all(|c| c.is_ascii_hexdigit()), "{build}");
        assert_ne!(engine(&["standard"]), engine(&["standard", "xml"]));
        assert_eq!(engine(&["standard"]), engine(&["standard"]));
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
        let programs = HEADER..u64_at(&bytes, 40).expect("where the index is") as usize;
        for n in 0..bytes.len() {
            assert!(open(bytes[..n].to_vec()).is_err(), "cut at {n}");
            let mut changed = bytes.clone();
            changed[n] ^= 0x20;
            match open(changed) {
                // A byte of the header or the index: the package is refused.
                Err(_) => assert!(!programs.contains(&n), "byte {n} of a program refused the package"),
                // A byte of a program: that file is not taken from it.
                Ok(p) => {
                    assert!(programs.contains(&n), "byte {n} of the header or index went unnoticed");
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

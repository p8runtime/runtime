//! The files save handler: `sess_<id>` files in session.save_path, locked
//! while the session is open.
//!
//! Reference: $PHP_SRC_PATH/ext/session/mod_files.c

use crate::state::{errno, notice, warn};
use p8_runtime::ext::Ctx;
use p8_runtime::stream::MAXPATHLEN;
use std::fs::{File, OpenOptions};
use p8_runtime::os::OsStrExt;
use std::path::{Path, PathBuf};

const FILE_PREFIX: &[u8] = b"sess_";

/// open(path, O_CREAT | O_RDWR | O_NOFOLLOW, mode)
#[cfg(unix)]
fn open_nofollow(path: &Path, mode: u32) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new().read(true).write(true).create(true).mode(mode).custom_flags(libc::O_NOFOLLOW).open(path)
}

/// open(path, O_CREAT | O_RDWR | O_NOFOLLOW): WASI has no permission bits,
/// and std reaches its no-follow lookup only through an unstable API, so a
/// link is refused before the open, with O_NOFOLLOW's ELOOP (WASI's 32).
#[cfg(target_os = "wasi")]
fn open_nofollow(path: &Path, _mode: u32) -> std::io::Result<File> {
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(std::io::Error::from_raw_os_error(32));
    }
    OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)
}

/// pread(fd, buf, size, 0)
#[cfg(unix)]
fn read_at_start(file: &File, buf: &mut [u8]) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buf, 0)
}

/// lseek(fd, 0, SEEK_SET) and read(), as mod_files does without pread
/// (std reaches WASI's pread only through an unstable API).
#[cfg(target_os = "wasi")]
fn read_at_start(mut file: &File, buf: &mut [u8]) -> std::io::Result<()> {
    use std::io::{Read, Seek};
    file.seek(std::io::SeekFrom::Start(0))?;
    file.read_exact(buf)
}

/// pwrite(fd, val, size, 0)
#[cfg(unix)]
fn write_at_start(file: &File, val: &[u8]) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, val, 0)
}

/// lseek(fd, 0, SEEK_SET) and write(), as mod_files does without pwrite.
#[cfg(target_os = "wasi")]
fn write_at_start(mut file: &File, val: &[u8]) -> std::io::Result<()> {
    use std::io::{Seek, Write};
    file.seek(std::io::SeekFrom::Start(0))?;
    file.write_all(val)
}

/// ps_files
pub(crate) struct Files {
    last_key: Option<Vec<u8>>,
    basedir: Vec<u8>,
    dirdepth: usize,
    st_size: u64,
    filemode: u32,
    file: Option<File>,
}

/// php_session_valid_key()
pub(crate) fn valid_key(key: &[u8]) -> bool {
    key.iter().all(|c| c.is_ascii_alphanumeric() || *c == b',' || *c == b'-') && !key.is_empty() && key.len() <= crate::PS_MAX_SID_LENGTH
}

fn path_create(data: &Files, key: &[u8]) -> Option<PathBuf> {
    if key.len() <= data.dirdepth || data.basedir.len() + 2 * data.dirdepth + key.len() + 5 + FILE_PREFIX.len() > MAXPATHLEN {
        return None;
    }
    let mut path = data.basedir.clone();
    path.push(b'/');
    for c in &key[..data.dirdepth] {
        path.push(*c);
        path.push(b'/');
    }
    path.extend_from_slice(FILE_PREFIX);
    path.extend_from_slice(key);
    Some(PathBuf::from(std::ffi::OsStr::from_bytes(&path)))
}

/// strtol(s, NULL, base): the digits at the start of `s`, and whether the
/// number was out of range.
fn strtol(s: &[u8], base: u32) -> (i64, bool) {
    let s = s.iter().position(|c| !c.is_ascii_whitespace()).map_or(&s[s.len()..], |i| &s[i..]);
    let (negative, digits) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let mut n: i64 = 0;
    let mut overflow = false;
    for c in digits {
        let Some(d) = (*c as char).to_digit(base) else { break };
        match n.checked_mul(i64::from(base)).and_then(|n| n.checked_add(i64::from(d))) {
            Some(v) => n = v,
            None => overflow = true,
        }
    }
    (if negative { -n } else { n }, overflow)
}

/// PS_OPEN_FUNC(files): the data for `save_path` ("[N;[MODE;]]path"), or
/// None after a warning.
pub(crate) fn open(ctx: &mut dyn Ctx, save_path: &[u8]) -> Option<Files> {
    let temp;
    let mut save_path = save_path;
    if save_path.is_empty() {
        temp = p8_runtime::os::temp_dir().into_os_string().into_encoded_bytes();
        save_path = &temp;
    }
    // split up the parameter: at most two ';' parts before the path
    let mut parts: Vec<&[u8]> = Vec::new();
    let mut last = save_path;
    while let Some(i) = last.iter().position(|c| *c == b';') {
        parts.push(&last[..i]);
        last = &last[i + 1..];
        if parts.len() > 1 {
            break;
        }
    }
    parts.push(last);
    let mut dirdepth = 0usize;
    let mut filemode = 0o600u32;
    if parts.len() > 1 {
        let (n, overflow) = strtol(parts[0], 10);
        if overflow {
            ctx.warn("The first parameter in session.save_path is invalid");
            return None;
        }
        dirdepth = n.max(0) as usize;
    }
    if parts.len() > 2 {
        let (n, overflow) = strtol(parts[1], 8);
        if overflow || !(0..=0o7777).contains(&n) {
            ctx.warn("The second parameter in session.save_path is invalid");
            return None;
        }
        filemode = n as u32;
    }
    let basedir = parts[parts.len() - 1].to_vec();
    Some(Files { last_key: None, basedir, dirdepth, st_size: 0, filemode, file: None })
}

impl Files {
    pub(crate) fn close_file(&mut self) {
        self.file = None;
    }

    /// ps_files_open(): the file of `key` open and locked (not if it is the
    /// one open already).
    fn open_file(&mut self, ctx: &mut dyn Ctx, key: &[u8]) {
        if self.file.is_some() && self.last_key.as_deref() == Some(key) {
            return;
        }
        self.last_key = None;
        self.close_file();
        if !valid_key(key) {
            warn(ctx, "Session ID is too long or contains illegal characters. Only the A-Z, a-z, 0-9, \"-\", and \",\" characters are allowed");
            return;
        }
        let Some(path) = path_create(self, key) else {
            warn(ctx, &format!("Failed to create session data file path. Too short session ID, invalid save_path or path length exceeds {MAXPATHLEN} characters"));
            return;
        };
        self.last_key = Some(key.to_vec());
        // O_NOFOLLOW keeps us from following evil symlinks
        match open_nofollow(&path, self.filemode) {
            Ok(file) => {
                let _ = file.lock();
                self.file = Some(file);
            }
            Err(e) => {
                warn(ctx, &format!("open({}, O_RDWR) failed: {}", String::from_utf8_lossy(path.as_os_str().as_bytes()), errno(&e)));
            }
        }
    }

    /// PS_READ_FUNC(files)
    pub(crate) fn read(&mut self, ctx: &mut dyn Ctx, key: &[u8]) -> Option<Vec<u8>> {
        self.open_file(ctx, key);
        let file = self.file.as_ref()?;
        let size = file.metadata().ok()?.len();
        self.st_size = size;
        if size == 0 {
            return Some(Vec::new());
        }
        let mut buf = vec![0u8; size as usize];
        match read_at_start(file, &mut buf) {
            Ok(()) => Some(buf),
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                warn(ctx, "Read returned less bytes than requested");
                None
            }
            Err(e) => {
                warn(ctx, &format!("Read failed: {}", errno(&e)));
                None
            }
        }
    }

    /// ps_files_write()
    pub(crate) fn write(&mut self, ctx: &mut dyn Ctx, key: &[u8], val: &[u8]) -> bool {
        // PS(id) may be changed by session_regenerate_id(): open_file()
        // opens the file again then.
        self.open_file(ctx, key);
        let Some(file) = self.file.as_ref() else { return false };
        // Truncate when the new data is smaller than the existing data.
        if (val.len() as u64) < self.st_size {
            let _ = file.set_len(0);
        }
        match write_at_start(file, val) {
            Ok(()) => true,
            Err(e) => {
                warn(ctx, &format!("Write failed: {}", errno(&e)));
                false
            }
        }
    }

    /// PS_UPDATE_TIMESTAMP_FUNC(files)
    pub(crate) fn update_timestamp(&mut self, ctx: &mut dyn Ctx, key: &[u8], val: &[u8]) -> bool {
        let Some(path) = path_create(self, key) else { return false };
        let touched = OpenOptions::new().write(true).open(&path).and_then(|f| f.set_modified(std::time::SystemTime::now()));
        if touched.is_err() {
            // New session ID: create the data file.
            return self.write(ctx, key, val);
        }
        true
    }

    /// PS_DESTROY_FUNC(files)
    pub(crate) fn destroy(&mut self, key: &[u8]) -> bool {
        let Some(path) = path_create(self, key) else { return false };
        if self.file.is_some() {
            self.close_file();
            if std::fs::remove_file(&path).is_err() {
                // A regenerated session not yet written to disk has no file.
                if path.exists() {
                    return false;
                }
            }
        }
        true
    }

    /// ps_files_key_exists()
    pub(crate) fn key_exists(&self, key: &[u8]) -> bool {
        path_create(self, key).is_some_and(|p| std::fs::metadata(p).is_ok())
    }

    /// PS_GC_FUNC(files): the number of files deleted, -1 when it cannot.
    pub(crate) fn gc(&self, ctx: &mut dyn Ctx, maxlifetime: i64) -> i64 {
        if self.dirdepth != 0 {
            // All cleanup is for an external entity (find -ctime x | xargs rm).
            return -1;
        }
        self.cleanup_dir(ctx, maxlifetime)
    }

    fn cleanup_dir(&self, ctx: &mut dyn Ctx, maxlifetime: i64) -> i64 {
        let dirname = String::from_utf8_lossy(&self.basedir).into_owned();
        let entries = match std::fs::read_dir(std::ffi::OsStr::from_bytes(&self.basedir)) {
            Ok(e) => e,
            Err(e) => {
                notice(ctx, &format!("ps_files_cleanup_dir: opendir({dirname}) failed: {}", errno(&e)));
                return -1;
            }
        };
        if self.basedir.len() >= MAXPATHLEN {
            notice(ctx, &format!("ps_files_cleanup_dir: dirname({dirname}) is too long"));
            return -1;
        }
        let now = std::time::SystemTime::now();
        let mut deleted = 0;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.as_bytes();
            if !name.starts_with(FILE_PREFIX) || name.len() + self.basedir.len() + 2 >= MAXPATHLEN {
                continue;
            }
            let path = entry.path();
            let age = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|m| now.duration_since(m).ok().map(|d| d.as_secs() as i64).or(Some(-1)));
            if age.is_some_and(|age| age > maxlifetime) {
                let _ = std::fs::remove_file(&path);
                deleted += 1;
            }
        }
        deleted
    }
}

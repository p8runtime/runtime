//! What p8 asks of the system that std reaches only through a platform's
//! own module: a file name's bytes, a file's stat fields, a canonical path.
//! On Unix these are std's Unix extensions, unchanged. On WASI
//! (wasm32-wasip1) they are what WASI has, and where it has nothing, what
//! PHP built against wasi-libc reports (its stat(), access() and
//! realpath()).

#[cfg(unix)]
pub use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(unix)]
pub use std::os::unix::fs::{FileTypeExt, MetadataExt};
#[cfg(target_os = "wasi")]
pub use std::os::wasi::ffi::{OsStrExt, OsStringExt};
#[cfg(target_os = "wasi")]
pub use wasi::{FileTypeExt, MetadataExt};

/// A path with every symbolic link, `.` and `..` resolved, which must
/// exist: realpath(3), what PHP's tsrm_realpath() gives.
#[cfg(unix)]
pub fn canonicalize(path: impl AsRef<std::path::Path>) -> std::io::Result<std::path::PathBuf> {
    std::fs::canonicalize(path)
}

/// A path with every symbolic link, `.` and `..` resolved, which must
/// exist. WASI has no realpath(); this walks the path one name at a time,
/// as PHP's own tsrm_realpath_r() does.
#[cfg(target_os = "wasi")]
pub fn canonicalize(path: impl AsRef<std::path::Path>) -> std::io::Result<std::path::PathBuf> {
    wasi::canonicalize(path.as_ref())
}

/// The system's temporary directory: TMPDIR, else /tmp.
#[cfg(unix)]
pub fn temp_dir() -> std::path::PathBuf {
    std::env::temp_dir()
}

/// The system's temporary directory: std has none on WASI (it panics), so
/// it is what php_get_temporary_directory() falls back to: TMPDIR, else
/// P_tmpdir, /tmp.
#[cfg(target_os = "wasi")]
pub fn temp_dir() -> std::path::PathBuf {
    std::env::var_os("TMPDIR").filter(|d| !d.is_empty()).map_or_else(|| "/tmp".into(), std::path::PathBuf::from)
}

/// The process's id: none on WASI, which has no processes.
#[cfg(unix)]
pub fn process_id() -> Option<u32> {
    Some(std::process::id())
}

/// The process's id: none on WASI, which has no processes.
#[cfg(target_os = "wasi")]
pub fn process_id() -> Option<u32> {
    None
}

/// The signal that ended a child process.
#[cfg(unix)]
pub fn exit_signal(status: &std::process::ExitStatus) -> Option<i32> {
    std::os::unix::process::ExitStatusExt::signal(status)
}

/// The signal that ended a child process: WASI has neither.
#[cfg(target_os = "wasi")]
pub fn exit_signal(_status: &std::process::ExitStatus) -> Option<i32> {
    None
}

#[cfg(unix)]
pub use std::os::unix::fs::{chown, lchown, symlink};

/// symlink(2): WASI has path_symlink, but std reaches it only through an
/// unstable API; it fails as wasi-libc does without it (ENOTSUP).
#[cfg(target_os = "wasi")]
pub fn symlink(_original: impl AsRef<std::path::Path>, _link: impl AsRef<std::path::Path>) -> std::io::Result<()> {
    Err(wasi::not_supported())
}

/// chown(2): WASI has no owners (ENOTSUP).
#[cfg(target_os = "wasi")]
pub fn chown(_path: impl AsRef<std::path::Path>, _uid: Option<u32>, _gid: Option<u32>) -> std::io::Result<()> {
    Err(wasi::not_supported())
}

/// lchown(2): WASI has no owners (ENOTSUP).
#[cfg(target_os = "wasi")]
pub fn lchown(_path: impl AsRef<std::path::Path>, _uid: Option<u32>, _gid: Option<u32>) -> std::io::Result<()> {
    Err(wasi::not_supported())
}

/// chmod(2).
#[cfg(unix)]
pub fn set_mode(path: impl AsRef<std::path::Path>, mode: u32) -> std::io::Result<()> {
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(mode))
}

/// chmod(2): WASI has no permission bits (ENOTSUP).
#[cfg(target_os = "wasi")]
pub fn set_mode(_path: impl AsRef<std::path::Path>, _mode: u32) -> std::io::Result<()> {
    Err(wasi::not_supported())
}

#[cfg(target_os = "wasi")]
mod wasi {
    use std::io;
    use std::path::{Component, Path, PathBuf};
    use std::time::UNIX_EPOCH;

    /// ENOTSUP, with the text wasi-libc's strerror() gives it.
    pub fn not_supported() -> io::Error {
        io::Error::new(io::ErrorKind::Unsupported, "Not supported")
    }

    /// The stat fields of a WASI file, as wasi-libc's stat() fills them:
    /// WASI has no owners and no permission bits (st_uid and st_gid are 0,
    /// st_mode holds the file type alone). What std does not reach (the
    /// device, the inode, the link count, the change time) is what PHP
    /// reports for a field the system lacks.
    pub trait MetadataExt {
        fn dev(&self) -> u64;
        fn ino(&self) -> u64;
        fn mode(&self) -> u32;
        fn nlink(&self) -> u64;
        fn uid(&self) -> u32;
        fn gid(&self) -> u32;
        fn rdev(&self) -> u64;
        fn size(&self) -> u64;
        fn atime(&self) -> i64;
        fn mtime(&self) -> i64;
        /// The status change time: std gives none on WASI; the modification
        /// time stands in.
        fn ctime(&self) -> i64;
        /// No st_blksize: PHP's -1 (as `u64::MAX`, cast back to -1).
        fn blksize(&self) -> u64;
        /// No st_blocks: PHP's -1 (as `u64::MAX`, cast back to -1).
        fn blocks(&self) -> u64;
    }

    fn secs(t: io::Result<std::time::SystemTime>) -> i64 {
        t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64)
    }

    impl MetadataExt for std::fs::Metadata {
        fn dev(&self) -> u64 {
            0
        }
        fn ino(&self) -> u64 {
            0
        }
        fn mode(&self) -> u32 {
            let t = self.file_type();
            if t.is_symlink() {
                0o120000
            } else if t.is_dir() {
                0o040000
            } else if t.is_file() {
                0o100000
            } else {
                0
            }
        }
        fn nlink(&self) -> u64 {
            1
        }
        fn uid(&self) -> u32 {
            0
        }
        fn gid(&self) -> u32 {
            0
        }
        fn rdev(&self) -> u64 {
            0
        }
        fn size(&self) -> u64 {
            self.len()
        }
        fn atime(&self) -> i64 {
            secs(self.accessed())
        }
        fn mtime(&self) -> i64 {
            secs(self.modified())
        }
        fn ctime(&self) -> i64 {
            secs(self.modified())
        }
        fn blksize(&self) -> u64 {
            u64::MAX
        }
        fn blocks(&self) -> u64 {
            u64::MAX
        }
    }

    /// The file types besides files, directories and links: WASI has no
    /// FIFOs, and std reaches its devices and sockets only through an
    /// unstable API, so none is seen as one (stat() says what it is not:
    /// a file, a directory or a link).
    pub trait FileTypeExt {
        fn is_fifo(&self) -> bool;
        fn is_char_device(&self) -> bool;
        fn is_block_device(&self) -> bool;
        fn is_socket(&self) -> bool;
    }

    impl FileTypeExt for std::fs::FileType {
        fn is_fifo(&self) -> bool {
            false
        }
        fn is_char_device(&self) -> bool {
            false
        }
        fn is_block_device(&self) -> bool {
            false
        }
        fn is_socket(&self) -> bool {
            false
        }
    }

    /// tsrm_realpath_r(): the path made absolute against the working
    /// directory, then each name in turn, a link replaced by its target.
    /// WASI reaches only what is under a preopened directory: a directory
    /// above one (`/home` above `/home/site`) cannot be looked at, and is
    /// taken as the plain directory it must be. What the path names must
    /// exist.
    pub fn canonicalize(path: &Path) -> io::Result<PathBuf> {
        // Builtins give an absolute path (resolved against the request's
        // directory, crate::cwd); a relative one is the process's.
        let start = if path.is_absolute() { path.to_path_buf() } else { crate::cwd::process_dir().join(path) };
        let mut pending: Vec<std::ffi::OsString> = Vec::new();
        push_names(&start, &mut pending);
        let mut out = PathBuf::from("/");
        // PHP gives up after this many links (ELOOP).
        let mut links = 0;
        while let Some(name) = pending.pop() {
            if name == "." {
                continue;
            }
            if name == ".." {
                out.pop();
                continue;
            }
            let next = out.join(&name);
            let link = std::fs::symlink_metadata(&next).is_ok_and(|m| m.file_type().is_symlink());
            if link {
                links += 1;
                if links > 32 {
                    return Err(io::Error::other("Too many levels of symbolic links"));
                }
                let target = std::fs::read_link(&next)?;
                if target.is_absolute() {
                    out = PathBuf::from("/");
                }
                push_names(&target, &mut pending);
            } else {
                out = next;
            }
        }
        if out.parent().is_some() {
            std::fs::symlink_metadata(&out)?;
        }
        Ok(out)
    }

    /// `path`'s names onto the stack, the first one on top.
    fn push_names(path: &Path, pending: &mut Vec<std::ffi::OsString>) {
        let names: Vec<std::ffi::OsString> = path
            .components()
            .filter_map(|c| match c {
                Component::Normal(n) => Some(n.to_os_string()),
                Component::ParentDir => Some("..".into()),
                Component::CurDir => Some(".".into()),
                Component::RootDir | Component::Prefix(_) => None,
            })
            .collect();
        pending.extend(names.into_iter().rev());
    }
}

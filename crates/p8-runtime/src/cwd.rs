//! The request's working directory (#459). A process has one working
//! directory; requests running side by side in one process each have
//! their own, as thread-safe PHP gives each thread its own
//! (`Zend/zend_virtual_cwd.c`, `VIRTUAL_DIR`). `chdir()` changes only the
//! request's ([`crate::ext::Ctx::set_cwd`]), and every builtin that hands
//! a path to the system resolves a relative one against it first, here.
//! Nothing in the engine reads or sets the process's directory but
//! [`process_dir`], once, when a request starts.

use crate::ext::Ctx;
use std::path::{Component, Path, PathBuf};

/// The process's working directory: where a request starts unless its
/// host says otherwise (PHP's `main_cwd_state`, read at startup). The one
/// place the engine reads it (`crates/p8/tests/safety.rs`). Empty when the
/// system cannot say (a removed directory): a relative path is then given
/// to the system as it is, as `virtual_file_ex` does with `cwd_length` 0.
pub fn process_dir() -> PathBuf {
    std::env::current_dir().unwrap_or_default()
}

/// `path` as the system must be given it for a request whose working
/// directory is `cwd`: an absolute path as it is, a relative one under
/// `cwd`, the way `virtual_file_ex` joins them before `open()`. Nothing is
/// folded and no link is resolved: the system resolves `..` and links
/// against the joined path exactly as it resolved the relative one against
/// the process's directory (what non-thread-safe PHP, the reference, does).
/// An empty path stays empty (PHP refuses it, `EINVAL`/`ENOENT`, and so does
/// the system), never the directory itself.
pub fn join(cwd: &Path, path: &Path) -> PathBuf {
    if path.as_os_str().is_empty() || path.is_absolute() || cwd.as_os_str().is_empty() {
        return path.to_path_buf();
    }
    cwd.join(path)
}

/// [`join`] against the running request's working directory: the one call
/// every builtin makes before giving a path to the system.
pub fn resolve(ctx: &dyn Ctx, path: impl AsRef<Path>) -> PathBuf {
    join(ctx.cwd(), path.as_ref())
}

/// `path` made absolute against `cwd` as `std::path::absolute` makes it
/// against the process's directory: [`join`]ed, `.` and repeated slashes
/// dropped, `..` kept (libxml2's xmlCanonicPath for a document's URL).
pub fn absolute(cwd: &Path, path: &Path) -> PathBuf {
    join(cwd, path).components().collect()
}

/// `expand_filepath()` (`main/fopen_wrappers.c`) for a path PHP shows
/// expanded (a zip archive's name, an xinclude base): [`join`]ed, then
/// `.`, `..` and repeated slashes folded by name, no link resolved
/// (`virtual_file_ex` with `CWD_EXPAND`). `..` above the root stays at it.
pub fn expand(cwd: &Path, path: &Path) -> PathBuf {
    let joined = join(cwd, path);
    let mut out = PathBuf::new();
    for part in joined.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if out.parent().is_some() {
                    out.pop();
                } else if !out.has_root() {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// A directory for `chdir()`: `path` resolved against `cwd`, every link
/// resolved (`virtual_chdir` uses `CWD_REALPATH`), and it must be a
/// directory (`php_is_dir_ok`). The error is the one chdir(2) gives
/// (`ENOENT`, `ENOTDIR`, `EACCES`).
pub fn change_to(cwd: &Path, path: &Path) -> std::io::Result<PathBuf> {
    if path.as_os_str().is_empty() {
        return Err(std::io::Error::from_raw_os_error(ENOENT));
    }
    let dir = crate::os::canonicalize(join(cwd, path))?;
    let meta = std::fs::metadata(&dir)?;
    if !meta.is_dir() {
        return Err(std::io::Error::from_raw_os_error(ENOTDIR));
    }
    // chdir(2) needs search permission on the directory: a directory no
    // one may search is refused as it refuses it.
    #[cfg(unix)]
    if std::os::unix::fs::PermissionsExt::mode(&meta.permissions()) & 0o111 == 0 {
        return Err(std::io::Error::from_raw_os_error(EACCES));
    }
    Ok(dir)
}

/// The errno PHP's chdir() warning shows for `e` ("... (errno 2)").
pub fn errno(e: &std::io::Error) -> i32 {
    e.raw_os_error().unwrap_or(match e.kind() {
        std::io::ErrorKind::PermissionDenied => EACCES,
        std::io::ErrorKind::NotADirectory => ENOTDIR,
        _ => ENOENT,
    })
}

#[cfg(unix)]
const ENOENT: i32 = 2;
#[cfg(unix)]
const EACCES: i32 = 13;
#[cfg(unix)]
const ENOTDIR: i32 = 20;
#[cfg(target_os = "wasi")]
const ENOENT: i32 = 44;
#[cfg(target_os = "wasi")]
const EACCES: i32 = 2;
#[cfg(target_os = "wasi")]
const ENOTDIR: i32 = 54;

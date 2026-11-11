//! Zend OPcache over p8's compile cache: its settings, its counters, the
//! rules it caches a file by, and the `CodeCache` view the `opcache_*`
//! functions reach it through. The cache itself is the Platform's
//! (`Compiler::cache`): kept files are shared by every isolate, and the
//! file cache on disk (`opcache.file_cache`) under it.
//!
//! While OPcache runs, it decides what is cached as PHP does: not a
//! blacklisted file, one changed in the last `file_update_protection`
//! seconds, or one larger than `max_file_size`; a cached file is checked
//! by its timestamp (`validate_timestamps`), at most every
//! `revalidate_freq` seconds, so a file changed in between is served as it
//! was, as PHP serves it. When OPcache is off (the CLI's default, without
//! `opcache.enable_cli`), p8 still keeps what it compiles, checked by its
//! source, and nothing of that is OPcache's to report.
//!
//! Reference: $PHP_SRC_PATH/ext/opcache/ZendAccelerator.c,
//! $PHP_SRC_PATH/ext/opcache/zend_accelerator_blacklist.c

use p8_runtime::opcache::{CacheStatus, CachedScript, CodeCache};
use std::path::Path;
use std::sync::Mutex;

/// OPcache's directives, as the host started with them (they are
/// PHP_INI_SYSTEM: fixed once the process runs).
#[derive(Debug, Clone)]
pub struct OpcacheSettings {
    pub enable: bool,
    pub enable_cli: bool,
    /// The host is the CLI (which needs enable_cli too); php -S is not.
    pub cli: bool,
    pub validate_timestamps: bool,
    pub revalidate_freq: i64,
    pub file_update_protection: i64,
    /// Bytes; 0: no limit.
    pub max_file_size: u64,
    pub file_cache: Option<String>,
    pub file_cache_only: bool,
    /// opcache.blacklist_filename: a glob of files listing paths.
    pub blacklist_filename: String,
}

impl OpcacheSettings {
    /// The settings `value` gives (an ini lookup: the host's -d settings,
    /// else the built-in default).
    pub fn from_ini(cli: bool, value: &dyn Fn(&str) -> Option<String>) -> OpcacheSettings {
        let on = |name: &str| value(name).is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "on" | "yes" | "true"));
        let int = |name: &str| value(name).map_or(0, |v| p8_runtime::ini::quantity(v.trim().as_bytes()));
        let file_cache = value("opcache.file_cache").filter(|v| !v.trim().is_empty());
        OpcacheSettings {
            enable: on("opcache.enable"),
            enable_cli: on("opcache.enable_cli"),
            cli,
            validate_timestamps: on("opcache.validate_timestamps"),
            revalidate_freq: int("opcache.revalidate_freq"),
            file_update_protection: int("opcache.file_update_protection"),
            max_file_size: int("opcache.max_file_size").max(0) as u64,
            file_cache,
            file_cache_only: on("opcache.file_cache_only"),
            blacklist_filename: value("opcache.blacklist_filename").unwrap_or_default(),
        }
    }

    /// accel_startup_ok: OPcache runs for this host.
    pub fn started(&self) -> bool {
        self.enable && (!self.cli || self.enable_cli)
    }
}

/// Counters that change as files are compiled and taken from the cache.
#[derive(Debug, Default)]
struct Counters {
    hits: u64,
    misses: u64,
    blacklist_misses: u64,
    last_restart_time: i64,
    manual_restarts: u64,
    restart_pending: bool,
}

/// What a kept file carries for OPcache.
#[derive(Debug, Clone, Default)]
pub(crate) struct ScriptStats {
    /// Cached by OPcache's rules (else kept by p8 only, checked by source).
    pub opcache: bool,
    pub hits: u64,
    pub last_used: i64,
    pub timestamp: Option<i64>,
    pub revalidate: i64,
    /// The memory it takes, counted the first time it is asked for (the
    /// entry never changes; a new compile is a new entry).
    pub memory: Option<u64>,
}

pub(crate) struct Opcache {
    pub settings: OpcacheSettings,
    blacklist: Vec<String>,
    start_time: i64,
    counters: Mutex<Counters>,
}

/// The time now, in Unix seconds.
pub(crate) fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// A file's modification time, in Unix seconds.
pub(crate) fn mtime(path: &str) -> Option<i64> {
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok()?;
    Some(modified.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64))
}

/// Why OPcache would not cache a file it compiles.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    Blacklisted,
    /// Changed too recently, too large, or no shared cache (file_cache_only).
    Other,
}

impl Opcache {
    pub fn new(settings: OpcacheSettings) -> Opcache {
        let blacklist = if settings.started() { load_blacklist(&settings.blacklist_filename) } else { Vec::new() };
        Opcache { settings, blacklist, start_time: now(), counters: Mutex::new(Counters::default()) }
    }

    fn counters(&self) -> std::sync::MutexGuard<'_, Counters> {
        self.counters.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// ZCSG(accelerator_enabled): whether new scripts go into the cache.
    /// opcache_reset() turns it off until the restart, at the next request;
    /// the request's own flag (ZCG, `settings.started()`) stays on, so the
    /// API keeps answering for the rest of the request.
    pub fn active(&self) -> bool {
        self.settings.started() && !self.counters().restart_pending
    }

    /// A request starts: a pending restart happens now (the cache is
    /// emptied by the caller when this says so).
    pub fn begin_request(&self) -> bool {
        let mut c = self.counters();
        if !c.restart_pending {
            return false;
        }
        // zend_reset_cache_vars(): the counters start again.
        c.restart_pending = false;
        c.hits = 0;
        c.misses = 0;
        c.blacklist_misses = 0;
        c.manual_restarts += 1;
        c.last_restart_time = now();
        true
    }

    pub fn schedule_restart(&self) {
        self.counters().restart_pending = true;
    }

    /// Whether OPcache caches `path` (of `size` bytes) once compiled now.
    pub fn refusal(&self, path: &str, size: usize) -> Option<Refusal> {
        // Only a file is cached by path (not eval'd code, stdin, or a
        // source an embedder compiles under a name of its own).
        if !Path::new(path).is_file() {
            return Some(Refusal::Other);
        }
        if is_blacklisted(&self.blacklist, path) {
            return Some(Refusal::Blacklisted);
        }
        if self.settings.file_cache_only {
            return Some(Refusal::Other);
        }
        if self.settings.max_file_size > 0 && size as u64 > self.settings.max_file_size {
            return Some(Refusal::Other);
        }
        // file_update_protection: a file changed in the last few seconds
        // may still be being written.
        if self.settings.file_update_protection > 0
            && let Some(t) = mtime(path)
            && now() - self.settings.file_update_protection < t
        {
            return Some(Refusal::Other);
        }
        None
    }

    /// The stats a file OPcache caches now starts with.
    pub fn new_stats(&self, path: &str) -> ScriptStats {
        let t = now();
        ScriptStats {
            opcache: true,
            hits: 0,
            last_used: t,
            timestamp: if self.settings.validate_timestamps { mtime(path) } else { None },
            revalidate: t + self.settings.revalidate_freq,
            memory: None,
        }
    }

    /// Whether a cached file is still valid (validate_timestamp_and_record):
    /// unchecked until its revalidate time, then by its timestamp.
    pub fn still_valid(&self, path: &str, stats: &mut ScriptStats) -> bool {
        if !self.settings.validate_timestamps {
            return true;
        }
        let t = now();
        if t < stats.revalidate {
            return true;
        }
        if mtime(path) != stats.timestamp {
            return false;
        }
        stats.revalidate = t + self.settings.revalidate_freq;
        true
    }

    pub fn hit(&self, stats: &mut ScriptStats) {
        stats.hits += 1;
        stats.last_used = now();
        self.counters().hits += 1;
    }

    pub fn miss(&self, blacklisted: bool) {
        let mut c = self.counters();
        if blacklisted {
            c.blacklist_misses += 1;
        } else {
            c.misses += 1;
        }
    }

    pub fn status(&self, scripts: Vec<CachedScript>) -> CacheStatus {
        let c = self.counters();
        CacheStatus {
            num_cached_scripts: scripts.len() as u64,
            used_memory: scripts.iter().map(|s| s.memory).sum(),
            scripts,
            hits: c.hits,
            misses: c.misses,
            blacklist_misses: c.blacklist_misses,
            start_time: self.start_time,
            last_restart_time: c.last_restart_time,
            manual_restarts: c.manual_restarts,
            restart_pending: c.restart_pending,
        }
    }

    pub fn blacklist(&self) -> Vec<String> {
        self.blacklist.clone()
    }
}

/// zend_accel_blacklist_load: every file the glob names, each line a path
/// (`;` comments, blank lines and surrounding quotes dropped), expanded
/// against the blacklist file's directory.
fn load_blacklist(pattern: &str) -> Vec<String> {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return Vec::new();
    }
    let mut files = glob_files(pattern);
    files.sort();
    let mut out = Vec::new();
    for file in files {
        let Ok(text) = std::fs::read(&file) else { continue };
        let dir = p8_runtime::os::canonicalize(&file).ok().and_then(|p| p.parent().map(Path::to_path_buf));
        for line in text.split(|&c| c == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            let mut line = line;
            while let Some(rest) = line.strip_prefix(b"\r") {
                line = rest;
            }
            if line.len() >= 2 && line[0] == b'"' && line[line.len() - 1] == b'"' {
                line = &line[1..line.len() - 1];
            }
            if line.is_empty() || line[0] == b';' {
                continue;
            }
            let entry = String::from_utf8_lossy(line).into_owned();
            out.push(expand(&entry, dir.as_deref()));
        }
    }
    out
}

/// expand_filepath_ex: a relative path joined to `dir` (else the process's
/// working directory: the blacklist is read at startup, before any
/// request), `.` and `..` folded, no link resolved.
fn expand(path: &str, dir: Option<&Path>) -> String {
    let joined = if Path::new(path).is_absolute() {
        path.to_string()
    } else {
        let base = dir.map(Path::to_path_buf).unwrap_or_else(p8_runtime::cwd::process_dir);
        format!("{}/{path}", base.to_string_lossy())
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            p => parts.push(p),
        }
    }
    format!("/{}", parts.join("/"))
}

/// The files a glob pattern names (`*` and `?` within one part).
fn glob_files(pattern: &str) -> Vec<String> {
    let path = Path::new(pattern);
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().map(|n| n.to_string_lossy().into_owned())) else { return Vec::new() };
    if !name.contains(['*', '?']) {
        return if path.is_file() { vec![pattern.to_string()] } else { Vec::new() };
    }
    let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| wildcard(name.as_bytes(), e.file_name().to_string_lossy().as_bytes(), false))
        .map(|e| e.path().to_string_lossy().into_owned())
        .collect()
}

/// Whether `text` matches `pattern`: `?` one character other than `/`,
/// `*` any run of them, `**` any run at all. `prefix`: the pattern need
/// only match the start of `text` (the blacklist's `^(…)` regexp).
fn wildcard(pattern: &[u8], text: &[u8], prefix: bool) -> bool {
    match pattern.split_first() {
        None => prefix || text.is_empty(),
        Some((b'*', rest)) => {
            let (any, rest) = match rest.split_first() {
                Some((b'*', r)) => (true, r),
                _ => (false, rest),
            };
            let mut i = 0;
            loop {
                if wildcard(rest, &text[i..], prefix) {
                    return true;
                }
                if i >= text.len() || (!any && text[i] == b'/') {
                    return false;
                }
                i += 1;
            }
        }
        Some((b'?', rest)) => text.first().is_some_and(|&c| c != b'/') && wildcard(rest, &text[1..], prefix),
        Some((c, rest)) => text.first() == Some(c) && wildcard(rest, &text[1..], prefix),
    }
}

/// zend_accel_blacklist_is_blacklisted.
fn is_blacklisted(entries: &[String], path: &str) -> bool {
    entries.iter().any(|e| wildcard(e.as_bytes(), path.as_bytes(), true))
}

/// The compile cache as OPcache presents it: the Platform's compiler.
impl CodeCache for crate::Compiler {
    fn started(&self) -> bool {
        self.opcache.as_ref().is_some_and(|o| o.settings.started())
    }

    fn status(&self, scripts: bool) -> CacheStatus {
        let Some(opcache) = &self.opcache else { return CacheStatus::default() };
        let mut list = Vec::new();
        if let Some(cache) = &self.cache {
            let mut kept = cache.lock().unwrap_or_else(|e| e.into_inner());
            let symbols = self.registry.symbols();
            for (path, kept) in kept.iter_mut().filter(|(_, k)| k.stats.opcache) {
                // What it takes: its source and its code (as the file cache
                // would write it).
                let memory = *kept.stats.memory.get_or_insert_with(|| (kept.source.len() + p8_bytecode::serial::encode(&kept.program, symbols).len()) as u64);
                list.push(CachedScript {
                    path: path.clone(),
                    hits: kept.stats.hits,
                    memory,
                    last_used: kept.stats.last_used,
                    timestamp: kept.stats.timestamp,
                    revalidate: kept.stats.revalidate,
                });
            }
        }
        let mut status = opcache.status(list);
        if !scripts {
            status.scripts.clear();
        }
        status
    }

    fn reset(&self) {
        if let Some(opcache) = &self.opcache {
            opcache.schedule_restart();
        }
    }

    fn invalidate(&self, path: &str, force: bool) -> bool {
        let Some(opcache) = &self.opcache else { return false };
        if !opcache.settings.started() {
            return false;
        }
        let real = p8_runtime::os::canonicalize(path).map_or_else(|_| path.to_string(), |p| p.to_string_lossy().into_owned());
        let mut found = Path::new(&real).exists();
        if let Some(files) = &self.files {
            files.forget(&real);
        }
        if let Some(cache) = &self.cache {
            let mut kept = cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(entry) = kept.get_mut(&real).filter(|k| k.stats.opcache) {
                found = true;
                let stale = opcache.settings.validate_timestamps && mtime(&real) != entry.stats.timestamp;
                if force || !opcache.settings.validate_timestamps || stale {
                    kept.remove(&real);
                }
            }
        }
        found
    }

    fn is_cached(&self, path: &str) -> bool {
        let Some(opcache) = &self.opcache else { return false };
        if !opcache.settings.started() || path.is_empty() {
            return false;
        }
        let real = p8_runtime::os::canonicalize(path).map_or_else(|_| path.to_string(), |p| p.to_string_lossy().into_owned());
        let Some(cache) = &self.cache else { return false };
        let mut kept = cache.lock().unwrap_or_else(|e| e.into_inner());
        kept.get_mut(&real).filter(|k| k.stats.opcache).is_some_and(|k| opcache.still_valid(&real, &mut k.stats))
    }

    fn in_file_cache(&self, path: &str) -> bool {
        let Some(files) = &self.files else { return false };
        let Ok(real) = p8_runtime::os::canonicalize(path) else { return false };
        let real = real.to_string_lossy().into_owned();
        let Ok(source) = std::fs::read(&real) else { return false };
        files.load(&source, &real, self.registry.symbols()).is_some()
    }

    fn blacklist(&self) -> Vec<String> {
        self.opcache.as_ref().map(Opcache::blacklist).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blacklist_patterns_match_as_php_does() {
        let entries = vec!["/tmp/path/?nocache.inc".to_string(), "/tmp/path/*/somedir".to_string(), "/a/b".to_string(), "/x/**/y".to_string()];
        assert!(is_blacklisted(&entries, "/tmp/path/Xnocache.inc"));
        assert!(!is_blacklisted(&entries, "/tmp/path/XYnocache.inc"));
        assert!(is_blacklisted(&entries, "/tmp/path/one/somedir/f.php"));
        assert!(!is_blacklisted(&entries, "/tmp/path/one/two/somedir"));
        // A prefix: everything under /a/b, and /a/bc too (PHP's regexp is unanchored at the end).
        assert!(is_blacklisted(&entries, "/a/b/c.php"));
        assert!(is_blacklisted(&entries, "/a/bc"));
        assert!(is_blacklisted(&entries, "/x/1/2/y"));
        assert!(!is_blacklisted(&entries, "/b/a"));
    }

    #[test]
    fn relative_entries_expand_against_the_blacklist_file() {
        assert_eq!(expand("./current.php", Some(Path::new("/srv/t"))), "/srv/t/current.php");
        assert_eq!(expand("blacklist.inc", Some(Path::new("/srv/t"))), "/srv/t/blacklist.inc");
        assert_eq!(expand("/tmp/../x/./y", None), "/x/y");
    }
}

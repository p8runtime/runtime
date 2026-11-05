//! Zend OPcache's view of the compile cache a host keeps: what the
//! `opcache_*` functions report and act on. The cache is p8's own (the
//! compiled files a Platform keeps for every isolate, and its file cache
//! on disk); this is the interface the standard extension reaches it by.
//!
//! Reference: $PHP_SRC_PATH/ext/opcache/zend_accelerator_module.c,
//! ext/opcache/zend_accelerator_hash.c (the key table's sizes)

use crate::ext::Ctx;

/// One cached file, as `opcache_get_status()['scripts']` lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct CachedScript {
    /// The resolved path it is cached under.
    pub path: String,
    /// Times it was taken from the cache instead of compiled.
    pub hits: u64,
    /// Bytes it takes: its source and its compiled code.
    pub memory: u64,
    /// When it was last compiled or taken from the cache (Unix seconds).
    pub last_used: i64,
    /// The file's modification time when it was compiled
    /// (validate_timestamps on).
    pub timestamp: Option<i64>,
    /// When its timestamp is checked again (revalidate_freq after the last check).
    pub revalidate: i64,
}

/// The cache's counters and contents (`opcache_get_status()`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CacheStatus {
    /// The cached files (empty unless asked for).
    pub scripts: Vec<CachedScript>,
    pub num_cached_scripts: u64,
    pub hits: u64,
    /// Compiles of files not in the cache (blacklisted ones excepted).
    pub misses: u64,
    /// Compiles of blacklisted files, which are never cached.
    pub blacklist_misses: u64,
    pub start_time: i64,
    pub last_restart_time: i64,
    pub manual_restarts: u64,
    /// opcache_reset() was called; the cache is emptied at the next request.
    pub restart_pending: bool,
    /// Bytes the cached files take.
    pub used_memory: u64,
}

/// The compile cache, as Zend OPcache presents it. Every method is about
/// the cache as it is now, shared by all of a Platform's isolates.
pub trait CodeCache: Send + Sync {
    /// Whether OPcache started for this host (accel_startup_ok): the CLI
    /// needs opcache.enable_cli as well as opcache.enable.
    fn started(&self) -> bool;
    /// Counters, and the cached files when `scripts`.
    fn status(&self, scripts: bool) -> CacheStatus;
    /// opcache_reset(): empty the cache when the next request starts
    /// (until then no new file is cached).
    fn reset(&self);
    /// opcache_invalidate(): drop `path`'s entry (and its file-cache
    /// entry) when it is stale or `force`; false when `path` is neither a
    /// file nor cached.
    fn invalidate(&self, path: &str, force: bool) -> bool;
    /// opcache_is_script_cached(): `path` is cached and still valid.
    fn is_cached(&self, path: &str) -> bool;
    /// opcache_is_script_cached_in_file_cache(): the file cache on disk
    /// holds `path` as it is now.
    fn in_file_cache(&self, path: &str) -> bool;
    /// The blacklist's entries, expanded (opcache_get_configuration()).
    fn blacklist(&self) -> Vec<String>;
}

/// A directive's current value as text ("" when unset).
pub fn ini_text(ctx: &dyn Ctx, name: &str) -> String {
    ctx.ini_get(name.as_bytes()).and_then(|v| v.to_php_string()).map(|s| String::from_utf8_lossy(s.as_bytes()).into_owned()).unwrap_or_default()
}

/// A boolean directive as OnUpdateBool reads it.
pub fn ini_on(ctx: &dyn Ctx, name: &str) -> bool {
    matches!(ini_text(ctx, name).trim().to_ascii_lowercase().as_str(), "1" | "on" | "yes" | "true")
}

/// ZCG(accelerator_enabled): OPcache started and is not turned off for
/// this request (opcache.enable may be set to 0 at run time).
pub fn accelerator_enabled(ctx: &dyn Ctx) -> bool {
    ctx.code_cache().is_some_and(|c| c.started()) && ini_on(ctx, "opcache.enable")
}

/// The hash table's size for opcache.max_accelerated_files: the first
/// prime of zend_accel_hash_init's list that holds that many keys.
pub fn max_cached_keys(files: i64) -> i64 {
    const PRIMES: &[i64] = &[5, 11, 19, 53, 107, 223, 463, 983, 1979, 3907, 7963, 16229, 32531, 65407, 130987, 262237, 524521, 1048793];
    PRIMES.iter().copied().find(|&p| p >= files).unwrap_or(1048793)
}

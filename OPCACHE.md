# Zend OPcache in p8

p8 reports Zend OPcache as loaded, as PHP 8.6 does: `extension_loaded('Zend OPcache')`
is true, `get_loaded_extensions(true)` is `["Zend OPcache"]`, and the `opcache_*`
functions are there. What they report and act on is p8's real compile cache: the
compiled files a Platform keeps for all its isolates (`Platform::keep_compiled`),
and the file cache on disk (`opcache.file_cache`). Nothing is a stand-in. A number
OPcache would report that p8 has no counterpart for is 0, or the row is not printed.

Code: `crates/p8/src/opcache.rs` (the cache's rules), `crates/p8-ext-standard/src/opcache.rs`
(the functions), `crates/p8-runtime/src/info.rs` (`accel_info`, the phpinfo() section).

## When it runs

As PHP 8.6 starts it (`accel_startup`):

- CLI (`php2 script.php`): only with `opcache.enable=1` **and** `opcache.enable_cli=1`.
  The default is `enable_cli=0`, so `opcache_get_status()` is false, and
  `opcache_compile_file()` says "Zend OPcache has not been properly started, can't compile file".
- Built-in server (`php2 -S`): with `opcache.enable=1` (the default). That server is not
  the CLI SAPI.
- `opcache_reset()` schedules a restart. The cache is emptied, and the counters
  start again, when the next request starts. Until then nothing new is cached, and the
  rest of the request still sees the cache as it was.

## Directives honoured

| directive | what p8 does |
|---|---|
| `opcache.enable`, `opcache.enable_cli` | start OPcache as above. Setting `opcache.enable=0` at run time turns it off for that request. |
| `opcache.validate_timestamps`, `opcache.revalidate_freq` | a cached file is checked against its mtime at most every `revalidate_freq` seconds. Until then (or never, with `validate_timestamps=0`) a changed file runs as it was cached, as in PHP. |
| `opcache.file_update_protection` | a file changed in the last N seconds is not cached. |
| `opcache.max_file_size` | a larger file is not cached. |
| `opcache.blacklist_filename` | the glob of blacklist files, with entries resolved against the blacklist file's folder; `?`, `*` and `**` as PHP matches them, by prefix. A blacklisted file is never cached and counts as a blacklist miss. |
| `opcache.file_cache` | p8's file cache on disk. |
| `opcache.file_cache_only` | nothing is kept in memory; status reports `file_cache_only`. |
| `opcache.file_cache_read_only` | file-cache entries are read and never written or removed. |
| `opcache.restrict_api` | a script whose path does not start with it gets the warning "Zend OPcache API is restricted by "restrict_api" configuration directive", and the function returns false. |
| `opcache.memory_consumption`, `opcache.max_accelerated_files` | reported: free memory is `memory_consumption` minus what the cached files take; max keys is the prime PHP sizes its table to. |

`opcache_invalidate()` drops the entry, and its file-cache entry, when it is forced, stale, or
`validate_timestamps` is off. `opcache_compile_file()` compiles and caches a file without
running it. `opcache_is_script_cached()` and `opcache_is_script_cached_in_file_cache()`
look at the real entries.

## Accepted, shown, not honoured

These are accepted and kept, and reported by `ini_get()`, `opcache_get_configuration()` and
phpinfo(). They change nothing in p8:

- **JIT**: `opcache.jit` (default `disable`), `opcache.jit_buffer_size` and every `opcache.jit_*`
  directive. p8 compiles no machine code. The status `jit` block is PHP's for a disabled JIT
  (all false or 0), phpinfo() says `JIT => Disabled`, and `opcache_jit_blacklist()` does nothing.
- **Shared memory**: `opcache.memory_consumption` is reported but not enforced. p8's cache is
  the process's memory, so it never fills, and `cache_full`, `oom_restarts` and
  `wasted_memory` stay 0. The same goes for `opcache.max_accelerated_files` (never full:
  `hash_restarts` 0), `opcache.max_wasted_percentage`, `opcache.force_restart_timeout`,
  `opcache.preferred_memory_model`, `opcache.lockfile_path`, `opcache.protect_memory` and
  `opcache.huge_code_pages`.
- **Interned strings**: `opcache.interned_strings_buffer`. p8 has no interned string buffer,
  so the status has no `interned_strings_usage` (PHP leaves it out without a buffer too).
- **Optimizer**: `opcache.optimization_level` and `opcache.opt_debug_level`. p8's compiler is
  its own and does not run Zend's optimizer passes, so it dumps no opcodes. phpinfo()'s
  "Optimization" row follows PHP's condition (started, and a non-zero level).
- **Keys and checks p8 does not need**: `opcache.use_cwd`, `opcache.revalidate_path`,
  `opcache.validate_permission`, `opcache.validate_root` and `opcache.dups_fix`. p8 keys a
  file by its resolved path, so two files never share an entry. Likewise
  `opcache.file_cache_consistency_checks`: a file-cache entry is always checked against
  the file's path and source, byte for byte, and against its checksum.
- **What is kept**: `opcache.save_comments` (p8 always keeps doc comments) and
  `opcache.record_warnings`. p8 raises a file's compile-time warnings and deprecations
  each time the file is loaded, cached or not, as PHP does with `record_warnings=1`.
  With PHP's default (0), they show only when the file is actually compiled.
- **Logging**: `opcache.error_log` and `opcache.log_verbosity_level`. p8 writes no OPcache log.
- **Other**: `opcache.enable_file_override`, and `opcache.preload` / `opcache.preload_user`
  (preloading is not done yet; see the follow-up issue).

## What PHP prints that p8 does not

- `opcache_get_status()`: no `interned_strings_usage`. `memory_usage` is what the cached
  files take, each counted as its source plus its compiled code as the file cache would
  write it, measured once per entry. It does not include PHP's fixed shared-memory overhead.
- phpinfo(): no "Shared memory model" row (there is no shared segment) and no "Interned
  Strings" rows.
- Startup checks PHP makes on the file cache's folder (an absolute, accessible path) are not
  made. An unusable folder is a cache that misses.

## Not like PHP, by design

`opcache.file_cache` works in p8's CLI without `opcache.enable_cli`, as it did before
OPcache was reported. PHP's CLI ignores it unless OPcache started. That is p8's ahead-of-time
compile cache, kept for speed.

## Tests that cannot pass

The `ext/opcache` tests that dump optimized opcodes (`opcache.opt_debug_level`) expect
Zend's own opcode listing, which p8 has no counterpart for. They are permanently out.
The JIT tests (`ext/opcache/tests/jit/`) that depend on JIT output or JIT-only
behaviour are out for the same reason.

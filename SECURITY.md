# Memory safety

p8 runs PHP without the class of bug behind most PHP CVEs:
use-after-free, double free and buffer overflows in C. Three rules keep it
that way.

## 1. No `unsafe` in this crate

`#![forbid(unsafe_code)]` is set on the library and on every binary, so the
compiler rejects `unsafe` blocks, functions and impls anywhere in our code.
`forbid` cannot be overridden by an inner `#[allow]`.

One crate is the exception, by decision (#402): `crates/p8-native`, the
boundary to the machine code of an executable `p8 compile --native` builds.
Calling generated code and exposing a helper to it cannot be done in safe
Rust, so that crate, and only it, holds `unsafe`: one file, every block
with its `SAFETY:` comment (clippy's `undocumented_unsafe_blocks` denied),
its doc comment the audit guide, the value layout it relies on checked at
startup. `crates/p8/tests/safety.rs` checks that every other p8 crate still
forbids `unsafe` and that the boundary stays one audited file. Machine code
is never loaded or mapped at run time: it exists only in executables linked
ahead of time.

PHP values live in a garbage-collected arena and are addressed by
generational handles. A handle that outlives its value is detected on use
and stops the request with a panic; it can never read whatever later reused
that slot. In Zend the same mistake is a use-after-free.

## 2. The default build links no C library

`cargo build` with default features compiles and links Rust only (plus the
operating system's own libraries). `tests/pure_rust_build.rs` fails if a
C-backed crate enters the default dependency graph.

Features that need C are opt-in. These flags are on the legacy package at the repository root. The `p8` crate does not use them.

| feature | C library | why it is not default |
|---|---|---|
| `openssl` | OpenSSL | `openssl_*` functions; a RustCrypto port is planned |
| `pdo-sqlite` | SQLite | legacy engine only; p8's `pdo_sqlite` is Turso, in Rust |
| `pdo-oci` | Oracle client | proprietary C client |
| `mysql-tls` | ring (C and assembly) | TLS for MySQL; the driver itself is Rust |
| `zip-c-codecs` | bzip2, zstd, xz | deflate, deflate64, lzma and AES stay on by default, in Rust |

Tracked for replacement (listed in the test; entries may only be removed):

| C library | used by | plan |
|---|---|---|
| PCRE2 | `preg_*` | a PCRE-compatible regex engine in Rust, measured against php-src's `ext/pcre` tests |

## 3. Process-wide C state is not used

Functions that would touch process-wide C state are implemented in Rust
instead: `setlocale()` and friends model the C locales PHP starts in,
`putenv()` changes a per-request environment that is passed to child
processes, and `crypt()` uses a Rust implementation. This also keeps one
request from affecting another in a threaded server.

## Reporting

Report security issues privately to the maintainers, not in public issues.

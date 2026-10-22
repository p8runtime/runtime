# Memory safety

php-rs exists to run PHP without the class of bug behind most PHP CVEs:
use-after-free, double free and buffer overflows in C. Three rules keep it
that way.

## 1. No `unsafe` in this crate

`#![forbid(unsafe_code)]` is set on the library and on every binary, so the
compiler rejects `unsafe` blocks, functions and impls anywhere in our code.
`forbid` cannot be overridden by an inner `#[allow]`.

PHP values live in a garbage-collected arena and are addressed by
generational handles. A handle that outlives its value is detected on use
and stops the request with a panic; it can never read whatever later reused
that slot. In Zend the same mistake is a use-after-free.

## 2. The default build links no C library

`cargo build` with default features compiles and links Rust only (plus the
operating system's own libraries). `tests/pure_rust_build.rs` fails if a
C-backed crate enters the default dependency graph.

Features that need C are opt-in:

| feature | C library | why it is not default |
|---|---|---|
| `openssl` | OpenSSL | `openssl_*` functions; a RustCrypto port is planned |
| `pdo-sqlite` | SQLite | no complete Rust SQLite yet |
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

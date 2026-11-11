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

p8 compiles and links Rust only (plus the operating system's own
libraries). `crates/p8/tests/safety.rs` fails if anything the `p8` crate
depends on builds or links C, and if a crate under `crates/` other than
`p8-native` does not forbid `unsafe`.

## 3. Process-wide C state is not used

Functions that would touch process-wide C state are implemented in Rust
instead: `setlocale()` and friends model the C locales PHP starts in,
`putenv()` changes a per-request environment that is passed to child
processes, and `crypt()` uses a Rust implementation. This also keeps one
request from affecting another in a threaded server.

## Reporting

Report security issues privately to the maintainers, not in public issues.

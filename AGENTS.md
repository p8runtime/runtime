# Repository guidelines

p8 is a PHP 8.6 engine in safe Rust. Its command is `p8`. Design decisions
live in the p8 APS (`p8runtime/aps`), cited as `APS N`.

## Layout

- `crates/p8`: the public embedding API; hosts use only this crate.
- `crates/p8-syntax`, `p8-compiler`, `p8-bytecode`, `p8-vmgen`, `p8-runtime`,
  `p8-interp`, `p8-pcre`, `p8-mysqlnd`, `p8-stubs`, `p8-ext-*`: the engine's
  internals.
- `crates/php-cli`: the `p8` binary, the only command: php's command line
  (`p8 script.php`, `-r`, `-S`, ...), plus `p8 compile` and `p8 serve`.
- `crates/php-test`: the `php-test` binary, which runs php-src's `.phpt`
  tests against `p8`. `crates/php-test/php-src/` explains the target and
  holds the scripts that build the reference PHP.
- `packages/vite-plugin-p8`: p8 built for `wasm32-wasip1`, as a Vite plugin.
- `docker/`: the `p8` container image.
- `bench/alloc-experiment`: the allocator measurements behind p8's allocator.

## Commands

```sh
cargo build --release --locked -p php-cli --bin p8
cargo build --release --locked -p php-test --bin php-test
cargo test --release --locked -p php-cli --test scripts --test file_cache --test package --test serve --test server --test command
cargo test --release --locked -p p8 --lib --test copies --test hash_counts --test release_counts --test slow_counts --test call_counts --test package --test api
cargo test --release --locked -p p8-bytecode -p p8-vmgen -p p8-runtime -p p8-interp -p php-test
cargo clippy --locked --workspace --all-targets --all-features
```

`php-cli`'s mysqli, pdo_mysql and pdo_pgsql tests are ignored unless a
database server is running (each file's header says which); the app
harnesses under `crates/php-cli/tests` (WordPress, MediaWiki, apps) need
Docker.

The php-src gate (php-src checked out at the commit p8-test-suite pins):

```sh
target/release/php-test ../php-src --suite ../p8-test-suite --php target/release/p8
```

## Rules

- `#![forbid(unsafe_code)]` on every crate; nothing p8 depends on builds or
  links C (`SECURITY.md`, checked by `crates/p8/tests/safety.rs`).
- `cargo fmt`; `cargo clippy --workspace --all-targets` adds no warning.
- Tests live in each crate's `tests/`.
- Commits: short imperative subjects.

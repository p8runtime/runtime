# Repository Guidelines

## Project Structure & Module Organization
- `crates/` holds **p8**, the new engine (docs/design/engine-v2.md), and its hosts:
  - `p8`: the public embedding API; hosts use only this crate.
  - `p8-syntax` (lexer, parser, AST; the legacy engine uses it too), `p8-compiler`, `p8-bytecode`, `p8-runtime` (values, operators, the extension ABI), `p8-interp`, `p8-ext-*` (builtins): p8's internals.
  - `php-cli`: the command-line host (`php2` until p8 replaces the legacy engine).
- `src/` is the legacy engine: VM/executor (`src/vm/`), runtime extensions (`src/runtime/`), builtins (`src/builtins/`). It is replaced by p8 at php-src parity; fixes go into p8.
- `src/bin/` hosts the legacy CLI entry points: `php` and `php-fpm`.
- `tests/` holds the legacy engine's integration tests; each p8 crate has its own `tests/`.
- `crates/p8-syntax/src/tests/` and its snapshots are not compiled yet (they predate this repo's layout).

## Build, Test, and Development Commands
- `cargo build`: compile the interpreter and binaries.
- `cargo test`: run the full Rust test suite (unit + integration).
- `cargo test --test array_functions`: run a single integration test file.
- `cargo run --bin php -- <script.php>`: execute a PHP script with the CLI binary.
- `cargo run --bin php-fpm`: start the FPM server implementation.

## Coding Style & Naming Conventions
- Follow standard Rust formatting (`cargo fmt`) with default rustfmt settings.
- Use `snake_case` for functions/modules, `CamelCase` for types, and `SCREAMING_SNAKE_CASE` for constants.
- Keep modules focused; prefer adding new builtins in `src/builtins/` and routing them through `src/builtins/mod.rs`.

## Testing Guidelines
- Add new integration tests under `tests/` using descriptive file names (for example, `tests/strict_types_eval.rs`).
- There is no explicit coverage target; keep tests representative of PHP compatibility behavior.

## Commit & Pull Request Guidelines
- Commit history is minimal (only an init commit), so no established convention exists. Use short, imperative subjects (for example, "Add array unpack tests").
- PRs should include: a brief description of behavior changes, tests run (`cargo test` or targeted tests), and any relevant PHP compatibility notes.

## Configuration & Dependencies Notes
- The project depends on several extension-related crates (OpenSSL, MySQL, PDO drivers). Keep new dependencies justified and scoped to the relevant extension module.

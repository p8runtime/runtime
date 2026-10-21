# php-rs

A PHP interpreter written in Rust. This project is currently in an experimental state.

## Origin and history

This is a fork of [wudi/php-rs](https://github.com/wudi/php-rs) by Di Wu (MIT). Its full history is kept here, and the work that followed in the deka runtime has been extracted onto the same timeline. Tags mark the points worth going back to:

| tag | date | state |
|---|---|---|
| `wudi-init` | 2025-12-31 | Di Wu's php-rs as forked |
| `gild-progress` | 2026-01-04 | our router, benchmark report and PHP-manual fixtures; **`main` starts here** |
| `deka-import` | 2026-01-10 | imported into deka; last pure-PHP point |
| `phpx-added` | 2026-01-25 | PHPX dialect added on top of the PHP VM |
| `last-full-extensions` | 2026-02-14 | last point with php-fpm, the router and the PDO/MySQLi/OpenSSL/Zip/hash builtins (needs the `deka-validation` crate) |
| `js-lowering` | 2026-02-21 | PHPX starts lowering to JavaScript |
| `last-vm` | 2026-02-22 | last commit with the Zend-style VM |
| `vm-deleted` | 2026-02-25 | VM removed during the move to JavaScript |
| `wudi-latest` | 2026-02-17 | upstream after the fork (mark-and-sweep GC) |

Branches: `history/deka` (the full deka timeline until the crate was removed), `upstream/wudi`, `archive/gild-progress`.

## Features

- Core PHP language support
- Standard library extensions (BCMath, JSON, MySQLi, PDO, OpenSSL, Zip, Zlib, etc.)
- CLI interface (`php`)
- FastCGI Process Manager (`php-fpm`)

## Limitations

- Fibers are not supported. The Gild runtime targets V8 and does not pause execution, so `Fiber::*` APIs are intentionally unavailable.
- Magic constants are not supported yet.

## Getting Started

### Prerequisites

- Rust (latest stable release)

### Building

Clone the repository and build using Cargo:

```bash
git clone https://github.com/addiscorp/php-rs.git
cd php-rs
cargo build --release
```

### Usage

#### CLI

Run a PHP script:

```bash
cargo run --bin php -- script.php
```

Interactive shell:

```bash
cargo run --bin php
```

#### FPM

Start the PHP-FPM server:

```bash
cargo run --bin php-fpm
```

## Testing

Run the Rust test suite:

```bash
cargo test
```

Legacy integration tests (`*.rs` files) now live under `test-old/`; the new `tests/` directory keeps the Bun-driven PHP fixtures and helpers.

## License

This project is licensed under the MIT License.

Created by AI

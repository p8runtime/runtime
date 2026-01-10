# php-rs

A PHP interpreter written in Rust. This project is currently in an experimental state.

## Features

- Core PHP language support
- Standard library extensions (BCMath, JSON, MySQLi, PDO, OpenSSL, Zip, Zlib, etc.)
- CLI interface (`php`)
- FastCGI Process Manager (`php-fpm`)

## Limitations

- Fibers are not supported. The php-rs runtime targets V8 and does not pause execution, so `Fiber::*` APIs are intentionally unavailable.
- Magic constants are not supported yet.

## Getting Started

### Prerequisites

- Rust (latest stable release)

### Building

Clone the repository and build using Cargo:

```bash
git clone https://github.com/wudi/php-rs.git
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

Legacy integration tests (`*.rs` files) now live under `test-old/`; the new `tests/php/` directory keeps the Bun-driven PHP fixtures and helpers.

## License

This project is licensed under the MIT License.

Created by AI

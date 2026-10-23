# php-rs

## Project Overview

`php-rs` is an experimental PHP interpreter written in Rust. It aims to support core PHP language features and standard library extensions, providing both a CLI interface (`php`) and a FastCGI Process Manager (`php-fpm`).

### Key Technologies

*   **Language:** Rust (Edition 2024)
*   **Core Dependencies:** `clap` (CLI), `tokio` (Async/FPM), `rustyline` (REPL), `chokidar` equivalent not used but filesystem watching implies typical dev workflows.
*   **Extensions:** `mysql`, `rusqlite`, `openssl`, `zip`, `flate2` (Zlib), `rust_decimal` (BCMath).

### Architecture

The project follows a classic interpreter architecture:

1.  **Parser (`crates/p8-syntax`):** Lexes and parses PHP source code into an AST.
2.  **Compiler (`src/compiler`):** Compiles the AST into bytecode chunks (`src/compiler/chunk.rs`).
3.  **Virtual Machine (`src/vm`):** Executes the bytecode instructions.
4.  **Runtime (`src/runtime`):** Manages memory, contexts, and extensions.
5.  **Builtins (`src/builtins`):** Implementations of PHP standard library functions (e.g., `array`, `json`, `math`).

## Building and Running

### Prerequisites

*   Rust (latest stable release)

### Build Commands

```bash
# Build the project in release mode
cargo build --release
```

### Run Commands

**CLI Interpreter:**

```bash
# Run a specific PHP script
cargo run --bin php -- path/to/script.php

# Start the Interactive Shell (REPL)
cargo run --bin php
```

**FastCGI Process Manager (FPM):**

```bash
# Start the PHP-FPM server
cargo run --bin php-fpm
```

### Testing

```bash
# Run all tests (unit and integration)
cargo test

# Run a specific integration test file
cargo test --test array_functions
```

## Development Conventions

*   **Code Style:** Adhere to standard Rust formatting (`cargo fmt`).
*   **Naming:**
    *   Functions/Modules: `snake_case`
    *   Types: `CamelCase`
    *   Constants: `SCREAMING_SNAKE_CASE`
*   **Module Structure:**
    *   New built-in functions should be added to `src/builtins/` and registered in `src/builtins/mod.rs`.
    *   Integration tests go in `tests/` with descriptive filenames (e.g., `tests/strict_types_eval.rs`).
    *   Parser tests are in `crates/p8-syntax/src/tests/` (not compiled yet).
*   **Dependencies:** New dependencies should be justified and scoped to their specific extension module (e.g., adding a crate only for `gd` support).

## Directory Overview

*   **`src/`**: Source code root.
    *   **`bin/`**: Entry points (`php`, `php-fpm`).
    *   **`builtins/`**: PHP standard library implementations.
    *   **`compiler/`**: AST to Bytecode compilation.
    *   **`core/`**: Core data structures (Values, Heap, Interner).
    *   **`fcgi/`**: FastCGI protocol implementation.
    *   **`parser/`**: Lexer and Parser.
    *   **`runtime/`**: Execution environment and extensions registry.
    *   **`vm/`**: Virtual Machine execution logic.
*   **`tests/`**: Integration tests covering various PHP features.

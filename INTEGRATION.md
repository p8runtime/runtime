# Gild + Deka Runtime Integration Plan

This document captures the recommended path to merge Gild into deka-runtime while keeping the "no spawning" and high performance constraints.

## Goal

Embed Gild as a library inside deka-runtime so that PHP-like scripts run in-process, alongside existing JS/TS handlers.

## Why This Approach

- Same language stack (Rust -> Rust). No IPC or process spawning.
- Matches deka-runtime performance model (single binary, pooled execution).
- Lets us reuse deka-runtime config, logging, request parsing, and response handling.

## Proposed Architecture

### 1) Split Gild into a core crate

- `gild-core`: compiler + VM + runtime (library crate)
- `gild-cli` / `gild-router`: remain as thin binaries in the Gild repo
- deka-runtime depends on `gild-core` only

### 2) Add a "language engine" interface in deka-runtime

- Define a `LanguageEngine` trait (or a parallel execution path) that can:
  - accept `RequestData`
  - execute a handler
  - return a response structure compatible with `IsolateResponse`

### 3) Routing + config

- Decide how PHP handlers are activated:
  - extension based (e.g., `.php` file)
  - explicit config flag (e.g., `runtime.toml` or CLI flag)
  - directory-based routing (e.g., `handlers/php/`)

### 4) Request/response bridge

- Map deka `RequestData` -> Gild runtime globals:
  - `$_SERVER`, `$_GET`, `$_POST`, `$_COOKIE`, `$_FILES`, input body
- Map Gild output -> deka `Response`:
  - status
  - headers
  - body

### 5) Deka ops inside Gild (optional but powerful)

- Add a small builtin surface so PHP can call deka modules:
  - `deka_env()`
  - `deka_fetch()`
  - `deka_postgres_query()`
  - `deka_redis_*()`
- Implement these via Rust calls to existing deka-runtime ops or shared modules.

## Suggested Integration Order

1. Extract `gild-core` and keep Gild CLI working.
2. Add a Gild engine in deka-runtime that can execute a single `.php` file.
3. Wire HTTP routing to run a PHP script and return a response.
4. Add deka builtin ops to Gild.
5. Expand to mixed JS/TS + PHP handling in one runtime.

## R&D Track (Optional)

- WASM target for Gild so it can run inside V8.
- This is a larger lift (host bindings, memory, debugging).
- Keep this parallel to the native integration path.

## Open Decisions

- How to select PHP handlers (extension vs config vs routing).
- Whether to run PHP side-by-side with JS/TS in a single server instance.
- How much PHP compatibility to preserve vs. Gild-specific behavior.


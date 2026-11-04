# p8 architecture

p8 is a PHP 8.4 runtime in safe Rust. This page is the map: how the parts fit,
the one rule about memory every change has to keep, and where things live.
The decisions behind it are the p8 APS (`p8runtime/aps`): APS 3 (engine and
hosts), APS 4 (values and memory), APS 5 (bytecode and execution) and APS 13
(program memory and request memory).

## The rule: program memory and request memory

| | Program memory | Request memory |
|---|---|---|
| What | bytecode, constants, literal strings, names, linked classes and their method tables | values, arrays, objects, globals, static properties, which classes a request has declared |
| Built | once per file version | during a request |
| Shape | frozen, `Send + Sync`, shared by every isolate on every core | owned by one request, copy-on-write |

**Request memory may point at program memory. It never copies it.** A literal
array or string is used where it is and copied only when a request changes it.
Before adding a `.clone()` of something that came from compiled code or a class
definition, ask whether a reference would do.

Execution state (frames and registers) belongs to a **task**, and switching tasks
swaps them, never copies them. That keeps fibers cheap, and it keeps native
async possible: its API is undecided (APS 13), so nothing may assume one request
per isolate or one line of execution per request.

Why: p8 was about 2.6× slower than PHP mostly because it copied program data
into every run, class and call. Safe Rust's own checks cost about 3%.

## The parts

```
crates/
  p8-syntax       lexer, parser, AST (from Di Wu's php-rs)
  p8-compiler     AST -> bytecode
  p8-bytecode     the compiled program: instructions, functions, class
                  definitions. Immutable once built (program memory).
  p8-runtime      values, strings, arrays, objects, operators, and the
                  extension API builtins are written against (request memory)
  p8-interp       runs bytecode for one request
  p8-ext-*        builtins (standard, pcre, pdo, mbstring, ...), written only
                  against p8-runtime's extension API
  p8              the public API hosts use: Platform, Isolate, Context
  php-cli         the command-line host
src/              the legacy engine (php-rs), kept compiling until p8 replaces it
```

Only `p8` is public API (APS 3). Its three levels:

- **Platform**: one per process. Extensions, the compiled-program cache, and
  shared tables. Everything in it is immutable and thread-safe.
- **Isolate**: one engine on one thread, reused request after request. Its
  buffers stay allocated, so a warm isolate starts a request without
  allocating them again.
- **Context**: one request. Its globals, declared functions and classes,
  output, and per-request state. Dropping it ends the request.

## Inside p8-interp

| file | what |
|---|---|
| `fast.rs` | the fast tier: an instruction loop for the common cases, plus frame entry and exit |
| `lib.rs` | `execute` (the loop between tiers) and `step` (every instruction in full generality) |
| `sites.rs` | call-site and property caches (feedback vectors) |
| `frames.rs` | the frame stack over one register file |
| `isolate.rs` | `Isolate`, `Context`, output |
| `classes.rs` | linking classes, method and property lookup, type checks |
| `fiber.rs`, `generators.rs` | fibers and generators |
| `callable.rs`, `handlers.rs`, `include.rs`, `throw.rs`, `reflection.rs`, ... | one PHP area each |

The fast tier is the hottest code. Anything added to `fast()` slows every
instruction slightly, because of how the compiler allocates registers across
the loop: three additions measured this week cost about 0.5% each, whether or
not they ran. Prefer making `step` cheaper (caches) to adding fast-tier cases,
and measure either way.

## Changing p8

- **Correctness** is checked against real PHP 8.4. The gates are php-src's own
  test suite (`php-test`, against `p8-test-suite`; a change may not lose a
  test), the app suites, `php-cli`'s script tests, and edge-case scripts whose
  output is diffed against PHP.
- **Speed** is measured as instruction counts (callgrind) on a Laravel test
  file and a WordPress page, base and change built with the same flags and run
  back to back. A change that doesn't win is reverted.
- `#![forbid(unsafe_code)]` everywhere, and no C dependencies. Memory safety is
  the point of p8 (SECURITY.md).
- Follow what PHP's source does (php-src 8.4) rather than patching around a
  test.

# p8

A PHP 8.6 engine in safe Rust. Hosts use the `p8` crate only. The command is `p8` (`crates/php-cli`): it runs PHP with php's command line (including `p8 -S`), and `p8 compile` / `p8 serve` compile and serve a project.

Not released yet. It is graded on php-src's own tests: https://p8-test-suite.addis.gg

## Layout

- `crates/p8`: the public embedding API.
- `crates/p8-syntax`, `p8-compiler`, `p8-bytecode`, `p8-runtime`, `p8-interp`, `p8-ext-*`: engine internals. Hosts do not depend on these.
- `crates/php-cli`: the `p8` command.
- `src/`: the legacy interpreter. It is replaced by p8 at php-src parity. `src/bin/` holds php-src's test runner (`php-test`), the legacy CLI its own tests run (`php`) and dev tools.

## Commands

```bash
cargo build -p php-cli --release
./target/release/p8 -r 'echo PHP_VERSION, "\n";'
cargo test -p p8
```

`#![forbid(unsafe_code)]` is on the engine. [SECURITY.md](SECURITY.md) is the rule. The license is MIT.

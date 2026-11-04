# p8

A PHP 8.6 engine in safe Rust. Hosts use the `p8` crate only. The command-line host is `php2` (`crates/php-cli`), including `php2 -S`.

Not released yet. It is graded on php-src's own tests: https://p8-test-suite.addis.gg

## Layout

- `crates/p8`: the public embedding API.
- `crates/p8-syntax`, `p8-compiler`, `p8-bytecode`, `p8-runtime`, `p8-interp`, `p8-ext-*`: engine internals. Hosts do not depend on these.
- `crates/php-cli`: the `php2` host.
- `src/`: the legacy interpreter. It is replaced by p8 at php-src parity. Its binaries live in `src/bin/`.

## Commands

```bash
cargo build -p php-cli --release
./target/release/php2 -r 'echo PHP_VERSION, "\n";'
cargo test -p p8
```

`#![forbid(unsafe_code)]` is on the engine. [SECURITY.md](SECURITY.md) is the rule. The license is MIT.

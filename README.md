# p8

A PHP 8.6 engine in safe Rust. It is embeddable. Its command is `p8`: it runs PHP with php's command line, and compiles and serves a project.

Not released yet. It is graded on php-src's own tests: [p8-test-suite.addis.gg](https://p8-test-suite.addis.gg).

## Run

```bash
cargo build -p php-cli --release
./target/release/p8 -r 'echo PHP_VERSION, "\n";'
./target/release/p8 -S 127.0.0.1:8080 -t .
```

A project compiled ahead and served from its package (`.p8/` beside the source):

```bash
./target/release/p8 compile path/to/site
./target/release/p8 serve path/to/site --listen 127.0.0.1:8000
```

## Test

php-src's own tests, with php-src checked out at the commit
[p8-test-suite](https://github.com/addiscorp/p8-test-suite) pins
([crates/php-test/php-src](crates/php-test/php-src/README.md)):

```bash
cargo build -p php-test --release
./target/release/php-test ../php-src --suite ../p8-test-suite --php target/release/p8
```

## Embed

Hosts talk only to the `p8` crate. `#![forbid(unsafe_code)]` is on the engine and the hosts. [SECURITY.md](SECURITY.md) is the rule.

```rust
let platform = p8::Platform::new(&[p8::extensions::STANDARD]);
let mut isolate = platform.new_isolate();
let script = platform.compile(b"<?php echo 1 + 2;", "a.php");
```

## See it

- [Playground](https://p8-tour.addis.gg), PHP in the browser.
- [WordPress](https://p8-wordpress.addis.gg), WordPress in the browser.

## License

MIT

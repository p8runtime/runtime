# p8

A PHP 8.6 engine in safe Rust. It is embeddable. The hosts are the `php2` command and its built-in server, `php2 -S`.

Not released yet. It is graded on php-src's own tests: [p8-test-suite.addis.gg](https://p8-test-suite.addis.gg).

## Run

```bash
cargo build -p php-cli --release
./target/release/php2 -r 'echo PHP_VERSION, "\n";'
./target/release/php2 -S 127.0.0.1:8080 -t .
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

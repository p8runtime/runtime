# Embedding

Hosts use the `p8` crate only. The crates behind it are not part of the API. `#![forbid(unsafe_code)]` is on the engine and the hosts. [SECURITY.md](SECURITY.md) is the rule.

```rust
struct Stdout;
impl p8::Host for Stdout {
    fn write(&mut self, bytes: &[u8]) {
        use std::io::Write;
        let _ = std::io::stdout().write_all(bytes);
    }
}

let platform = p8::Platform::new(&[p8::extensions::STANDARD]); // once per process
let mut isolate = platform.new_isolate();                      // once per thread
let script = platform.compile(b"<?php echo 1 + 2;", "a.php");
let mut host = Stdout;
let mut context = isolate.new_context(&mut host);              // once per request
let status = match &script {
    Ok(script) => context.run(script),
    Err(error) => context.fail(error),
};
```

The command-line host is `p8`, in `crates/php-cli`.

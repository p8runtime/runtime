# php-src: the compatibility target

p8's correctness gate (APS 7) is php-src's own test suite, at the version p8
targets (PHP 8.4.27).

## Files

- `target.txt` lists the **16,077 tests that real PHP 8.4.27 passes**.
  - Real PHP runs the suite with php-src's `run-tests.php` in the official
    `php:8.4-cli` image.
  - A test counts as passing when its JUnit entry has no failure, error or
    skip.
  - "100%" for p8 means passing every test on this list.
- `make-target.sh` regenerates `target.txt` from a php-src checkout at the
  same tag.
- `p8-passes.txt` is the target tests p8 passes today, sorted. **A change may
  not lose a test from it.** A change that gains tests updates it, and the PR
  shows the difference.

## Running it

```sh
cargo build --release --bin php-test
cargo build --release -p php-cli --bin php2
target/release/php-test ../php-src -r -q --php target/release/php2 \
  --target tests/php-src/target.txt --passes .tmp/p8-passes.txt
diff tests/php-src/p8-passes.txt .tmp/p8-passes.txt
```

The run writes into the php-src checkout, so reset it afterwards:
`git checkout -- . && git clean -fd` in php-src.

`php-test` spawns the engine once per test, as php-src's own runner does, so
one test can't crash or stall the others. Each test has a timeout.

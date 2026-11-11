# php-src: the compatibility target

p8's correctness gate (APS 7) is php-src's own test suite, at the version p8
targets: PHP 8.6 (Sami, 4 Nov 2026: "p8's compatibility target moves from
PHP 8.4 to PHP 8.6"), at 8.6.0RC5 until 8.6.0 is released on 19 Nov 2026. The
target and p8's recorded results live in
[addiscorp/p8-test-suite](https://github.com/addiscorp/p8-test-suite):

- `target.txt`: the **18,121 tests that real PHP 8.6.0RC5 passes**. "100%" for
  p8 means passing every one. `make-target.sh` here regenerates it: it builds
  that PHP from the suite's `php-src.json` archive with `php-src.Dockerfile`
  (the official `php:<version>-cli` recipe plus mbstring, which the official
  8.6 images no longer include), runs php-src's `run-tests.php` with it, runs
  the failures again one at a time, and counts a test when a JUnit entry has
  no failure, error or skip. Moving to a new PHP release is: update
  `php-src.json` in the suite, run `make-target.sh`, record a run. The app
  harnesses (`crates/php-cli/tests`: the WordPress gate, the WordPress and
  MediaWiki suites, `apps/run.sh`) compare p8 with the same build:
  `reference.sh` builds it for both, and `php-ext.Dockerfile` adds what
  their images need to install extensions.
- `results.txt` and `run.json`: p8's state for each target test at a `main`
  commit. **A change may not lose a test that passes there.**
- `php-src.json`: the php-src release, which the checkout must be at.

p8-test-suite.addis.gg draws the results.

## Running it

Check out p8-test-suite and php-src (at the suite's `php-src.json` commit)
next to this repository, then:

```sh
cargo build --release --bin php-test
cargo build --release -p php-cli --bin p8
target/release/php-test ../php-src --suite ../p8-test-suite --php target/release/p8
```

Every target test runs, compared with the suite's results: the tests gained
and lost, and a table of the areas that changed. A lost test makes it exit
1. A full run takes a few minutes; for quick feedback narrow it:

- `-f ext/zlib`: tests whose path contains the text (an area, one test).
- `--only fail` (or `pass`, `skip`): tests the suite records in that state.
  `--only pass` is a quick check that nothing was lost.
- `--outputs .tmp/out`: for every test that doesn't pass, what it expected
  and what p8 printed, as `.tmp/out/<test path>.exp` and `.out`.
- `-v`: print the expected and actual output of failures inline.

Tests not run keep their recorded state in the target total.

`--write` records a full run in the suite's `results.txt` and `run.json`,
against this checkout's HEAD (it must be clean): a new p8-test-suite release
is a pull request with that change, tagged `results-<commit>` once merged.

## How tests run

- Like php-src's `run-tests.php -j`: a worker takes a batch of up to 32
  consecutive tests and runs them in order (neighbouring tests share files);
  `CONFLICTS` files and `--CONFLICTS--` sections keep tests with the same key
  apart; tests with the key `all` run last, alone.
- `php-test` spawns the engine once per test, as php-src's runner does, so
  one test can't crash or stall the others. Each test has a timeout and caps
  on memory and processes (`-j` at most half the cores).
- Tests write into the php-src checkout, so `--suite` resets it
  (`git checkout -- . && git clean -fd`) before and after a run.

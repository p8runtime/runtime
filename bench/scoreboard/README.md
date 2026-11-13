# Scoreboard: main's WordPress and Laravel counts

`results.tsv` is p8's standing against PHP 8.6 on two real applications, one
row per measured main commit. A row is only ever appended by `run.sh`; no
number in it is typed by hand.

## What it measures

Machine instructions ("I refs") under valgrind's callgrind, whole process, on
the measuring host (addis, x86-64 Linux), with its app kit
`~/p8-perf/apps/count.sh` (the kit's README says how it works):

- **WordPress:** one front-page request, `cd wp && <binary> render.php`, the
  database put back to the same state before each run. The page must be byte
  for byte PHP's.
- **Laravel:** `vendor/bin/phpunit tests/Support/SupportCollectionTest.php`
  (782 tests). The report must be PHP's, or one of the kit's known reports.
- **Execution only:** the whole run minus everything under the compiler's
  entry (`p8::compile`; PHP's `compile_file` and `compile_string`), so
  compiling is excluded. The ratio columns are p8 over PHP on this figure.
- **Whole run:** p8's full count, compiling included.

The p8 binary is the calls kit's recipe (`~/p8-perf/calls/build.sh profiling`:
release with debug information, the host's nixpkgs toolchain), named
`~/p8-perf/calls/bin/main-<first 8 of sha>`. PHP is the kit's PHP 8.6
(the `PHP=` line of `count.sh`), counted in the same session as p8. A count
is not exact: two runs of one binary differ by up to about 0.35M on
WordPress and 1.0M on Laravel.

## Columns

date (UTC), sha, WordPress execution p8, WordPress execution PHP, ratio,
Laravel execution p8, Laravel execution PHP, ratio, WordPress whole run p8,
Laravel whole run p8, the p8 count label, the PHP count label. The raw files
of each count are on the host in `~/p8-perf/apps/out/<label>/`.

## Run it

From a checkout, on a machine that can `ssh addis`, after main's binary has
been built on the host:

```sh
bench/scoreboard/run.sh <sha> [host]
```

It counts the binary once as `score-<UTC date>-<sha8>` and PHP once as
`score-<UTC date>-<sha8>-php` (about 1.5 minutes, waiting its turn on the
kit's lock), reads the numbers from the counts, and appends one row. A label
already counted that day is read back rather than counted again. It does not
build. On a missing binary, a failed count or a page/report that is WRONG it
appends nothing and exits non-zero. Commit the new row.

# Gild Benchmark (Local)

This benchmark compares the Gild router against containerized nginx+php-fpm and apache+php-fpm.

## Bench Script

`bench.php` burns CPU for N milliseconds using a busy loop. Default: 8ms.

Example:

```
http://127.0.0.1:8541/bench.php?ms=8
```

## 1) Gild Router

Build:

```
cargo build --release --features router --bin php --bin gild-router
```

Run:

```
GILD_NATIVE_BIN=target/release/php GILD_DOC_ROOT=bench target/release/gild-router
```

Gild listens on `http://127.0.0.1:8541`.

## 2) Docker baselines (nginx + apache)

The perf script uses Docker to spin up:

- nginx + php-fpm on port `8542`
- apache + php-fpm on port `8543`

## 3) Markdown perf report

Run the harness (starts containers automatically):

```
./bench/http-perf.sh
```

Config options:

- `RUNTIMES=gild,nginx,apache`
- `DURATION=10`
- `CONCURRENCY=50`
- `WORK_MS=8`
- `REPORT_PATH=bench/HTTP-REPORT.md`

Example:

```
RUNTIMES=gild,nginx DURATION=15 CONCURRENCY=200 WORK_MS=8 ./bench/http-perf.sh
```

## Notes

- Requires Docker for nginx/apache baselines.
- Gild router logs: `bench/gild-router.log`.
- Extension scan helper: `bench/scan-framework-extensions.py` (regenerates `EXTENSIONS_FRAMEWORKS.md`).
- Packagist scan helper: `bench/scan-packagist-extensions.py` (regenerates `EXTENSIONS_PACKAGES.md`).

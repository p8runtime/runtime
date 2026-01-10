# php-router Benchmark (Local)

This benchmark compares the php-router against containerized nginx+php-fpm and apache+php-fpm.

## Bench Script

`bench.php` burns CPU for N milliseconds using a busy loop. Default: 8ms.

Example:

```
http://127.0.0.1:8541/bench.php?ms=8
```

## 1) php-router

Build:

```
cargo build --release --features router --bin php --bin php-router
```

Run:

```
PHP_NATIVE_BIN=target/release/php PHP_ROUTER_DOC_ROOT=bench target/release/php-router
```

php-router listens on `http://127.0.0.1:8541`.

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

- `RUNTIMES=php-router,nginx,apache`
- `DURATION=10`
- `CONCURRENCY=50`
- `WORK_MS=8`
- `REPORT_PATH=bench/HTTP-REPORT.md`

Example:

```
RUNTIMES=php-router,nginx DURATION=15 CONCURRENCY=200 WORK_MS=8 ./bench/http-perf.sh
```

## Notes

- Requires Docker for nginx/apache baselines.
- php-router logs: `bench/php-router.log`.
- Extension scan helper: `bench/scan-framework-extensions.py` (regenerates `EXTENSIONS_FRAMEWORKS.md`).
- Packagist scan helper: `bench/scan-packagist-extensions.py` (regenerates `EXTENSIONS_PACKAGES.md`).

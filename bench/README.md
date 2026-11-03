# HTTP benchmark

`bench.php` burns CPU for N milliseconds using a busy loop. Default work is 8ms.

`./bench/http-perf.sh` compares a local PHP router with containerized nginx+php-fpm (port 8542) and apache+php-fpm (port 8543). It writes `bench/HTTP-REPORT.md`.

Docker is required for the nginx and apache baselines.

- `DURATION` (default 10)
- `CONCURRENCY` (default 50)
- `WORK_MS` (default 8)
- `REPORT_PATH` (default `bench/HTTP-REPORT.md`)

`bench/scan-framework-extensions.py` regenerates `EXTENSIONS_FRAMEWORKS.md`.
`bench/scan-packagist-extensions.py` regenerates `EXTENSIONS_PACKAGES.md`.

# HTTP Perf Report

Generated: 2026-01-04T05:39:09Z
Duration: 20s
Concurrency: 500
Work: 10ms

| Runtime | URL | Total | Errors | RPS | P50 (ms) | P95 (ms) |
| --- | --- | --- | --- | --- | --- | --- |
| php-router | http://127.0.0.1:8541/bench.php?ms=10 | 24106 | 20 | 1205.3 | 342.89 | 608.76 |
| nginx+php-fpm | http://127.0.0.1:8542/bench.php?ms=10 | 9823 | 0 | 491.15 | 1030.45 | 1255.32 |
| apache+php-fpm | http://127.0.0.1:8543/bench.php?ms=10 | 16113 | 33210 | 805.65 | 244.43 | 740.39 |

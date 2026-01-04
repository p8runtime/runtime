#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BENCH_DIR="${ROOT_DIR}/bench"
NODE_BIN="${NODE_BIN:-bun}"
RUNTIMES="${RUNTIMES:-gild,nginx,apache}"
DURATION="${DURATION:-10}"
CONCURRENCY="${CONCURRENCY:-50}"
WORK_MS="${WORK_MS:-8}"
REPORT_PATH="${REPORT_PATH:-$BENCH_DIR/HTTP-REPORT.md}"

PORT_GILD="${PORT_GILD:-8541}"
PORT_NGINX="${PORT_NGINX:-8542}"
PORT_APACHE="${PORT_APACHE:-8543}"

GILD_DOC_ROOT="${BENCH_DIR}"
GILD_BIN="${ROOT_DIR}/target/release/gild-router"
GILD_NATIVE_BIN="${ROOT_DIR}/target/release/php"

DOCKER_BIN="${DOCKER_BIN:-docker}"
COMPOSE_FILE="${COMPOSE_FILE:-$BENCH_DIR/docker-compose.yml}"

PIDS=()
STARTED_GILD=0
STARTED_DOCKER=0
GILD_PID=""

cleanup() {
  for pid in "${PIDS[@]:-}"; do
    kill "$pid" >/dev/null 2>&1 || true
  done
  if [[ $STARTED_DOCKER -eq 1 ]]; then
    "$DOCKER_BIN" compose -f "$COMPOSE_FILE" down >/dev/null 2>&1 || true
  fi
  if [[ $STARTED_GILD -eq 1 && -n "${GILD_PID}" ]]; then
    kill "$GILD_PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

ensure_command() {
  local label="$1"
  local bin="$2"
  if command -v "$bin" >/dev/null 2>&1; then
    return 0
  fi
  if [[ -x "$bin" ]]; then
    return 0
  fi
  echo "[perf:${label}] missing binary: ${bin}" >&2
  exit 1
}

ensure_js_runtime() {
  if command -v bun >/dev/null 2>&1; then
    NODE_BIN="bun"
    return 0
  fi
  echo "[perf] bun not found; install bun to run the perf harness" >&2
  exit 1
}

wait_for_url() {
  local url="$1"
  for _ in $(seq 1 50); do
    if curl -sS "$url" >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.1
  done
  return 1
}

port_listening() {
  local port="$1"
  if command -v lsof >/dev/null 2>&1; then
    lsof -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1
    return $?
  fi
  return 1
}

start_gild() {
  if port_listening "$PORT_GILD"; then
    return
  fi
  if [[ ! -x "$GILD_BIN" || ! -x "$GILD_NATIVE_BIN" ]]; then
    echo "[perf:gild] binaries not found; building..." >&2
    (cd "$ROOT_DIR" && cargo build --release --features router --bin gild-router --bin php)
  fi
  GILD_NATIVE_BIN="$GILD_NATIVE_BIN" GILD_DOC_ROOT="$GILD_DOC_ROOT" "$GILD_BIN" \
    > "${BENCH_DIR}/gild-router.log" 2>&1 &
  GILD_PID=$!
  STARTED_GILD=1
  sleep 0.4
  if ! kill -0 "$GILD_PID" >/dev/null 2>&1; then
    echo "[perf:gild] failed to start. See ${BENCH_DIR}/gild-router.log" >&2
    return 1
  fi
}

start_docker_services() {
  if [[ $STARTED_DOCKER -eq 1 ]]; then
    return 0
  fi
  ensure_command "docker" "$DOCKER_BIN"
  if [[ ! -f "$COMPOSE_FILE" ]]; then
    echo "[perf:docker] compose file not found: $COMPOSE_FILE" >&2
    return 1
  fi
  "$DOCKER_BIN" compose -f "$COMPOSE_FILE" up -d >/dev/null 2>&1 || {
    echo "[perf:docker] failed to start containers" >&2
    return 1
  }
  STARTED_DOCKER=1
  sleep 0.6
}

run_perf() {
  local label="$1"
  local url="$2"
  echo "[perf:${label}] ${url}" >&2
  set +e
  local out
  out=$("$NODE_BIN" "$BENCH_DIR/http-perf.mjs" --url "$url" --duration "$DURATION" --concurrency "$CONCURRENCY" 2>&1)
  local status=$?
  set -e
  if [[ $status -ne 0 ]]; then
    echo "[perf:${label}] load test failed (exit ${status})" >&2
    echo "$out" >&2
    return $status
  fi
  echo "$out"
}

write_report_header() {
  local now
  now="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
  cat <<HEADER > "$REPORT_PATH"
# HTTP Perf Report

Generated: ${now}
Duration: ${DURATION}s
Concurrency: ${CONCURRENCY}
Work: ${WORK_MS}ms

| Runtime | URL | Total | Errors | RPS | P50 (ms) | P95 (ms) |
| --- | --- | --- | --- | --- | --- | --- |
HEADER
}

append_report_row() {
  local label="$1"
  local json="$2"
  local row
  row="$(echo "$json" | "$NODE_BIN" -e '
    const fs = require("fs");
    const input = fs.readFileSync(0, "utf8").trim();
    if (!input) process.exit(1);
    const data = JSON.parse(input);
    const cells = [
      data.url,
      data.total,
      data.errors,
      data.rps,
      data.p50_ms,
      data.p95_ms,
    ];
    process.stdout.write(`| ${data.runtime || ""} | ${cells.join(" | ")} |`);
  ' 2>/dev/null || true)"
  if [[ -z "$row" ]]; then
    echo "| ${label} | ${json} | - | - | - | - | - |" >> "$REPORT_PATH"
    return
  fi
  row="${row/|  |/| ${label} |}"
  echo "${row}" >> "$REPORT_PATH"
}

append_report_error() {
  local label="$1"
  local message="$2"
  echo "| ${label} | ${message} | - | - | - | - | - |" >> "$REPORT_PATH"
}

write_report_header

if [[ "$RUNTIMES" != *"gild"* && "$RUNTIMES" != *"nginx"* && "$RUNTIMES" != *"apache"* ]]; then
  echo "[perf] no runtimes selected (RUNTIMES=$RUNTIMES)" >&2
  exit 1
fi

ensure_js_runtime

if [[ "$RUNTIMES" == *"gild"* ]]; then
  start_gild
  url="http://127.0.0.1:${PORT_GILD}/bench.php?ms=${WORK_MS}"
  wait_for_url "$url"
  if result="$(run_perf "gild" "$url")"; then
    append_report_row "gild" "$result"
  else
    append_report_error "gild" "error"
  fi
fi

if [[ "$RUNTIMES" == *"nginx"* || "$RUNTIMES" == *"apache"* ]]; then
  if ! start_docker_services; then
    if [[ "$RUNTIMES" == *"nginx"* ]]; then
      append_report_error "nginx+php-fpm" "error"
    fi
    if [[ "$RUNTIMES" == *"apache"* ]]; then
      append_report_error "apache+php-fpm" "error"
    fi
  fi
fi

if [[ "$RUNTIMES" == *"nginx"* ]]; then
  url="http://127.0.0.1:${PORT_NGINX}/bench.php?ms=${WORK_MS}"
  if wait_for_url "$url"; then
    if result="$(run_perf "nginx+php-fpm" "$url")"; then
      append_report_row "nginx+php-fpm" "$result"
    else
      append_report_error "nginx+php-fpm" "error"
    fi
  else
    append_report_error "nginx+php-fpm" "error"
  fi
fi

if [[ "$RUNTIMES" == *"apache"* ]]; then
  url="http://127.0.0.1:${PORT_APACHE}/bench.php?ms=${WORK_MS}"
  if wait_for_url "$url"; then
    if result="$(run_perf "apache+php-fpm" "$url")"; then
      append_report_row "apache+php-fpm" "$result"
    else
      append_report_error "apache+php-fpm" "error"
    fi
  else
    append_report_error "apache+php-fpm" "error"
  fi
fi

echo "report written to $REPORT_PATH"

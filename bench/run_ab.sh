#!/usr/bin/env bash
set -euo pipefail

N=10000
C=200
MS=8
PHP_ROUTER_URL="http://127.0.0.1:8541/bench.php"
NGINX_URL="http://127.0.0.1:8542/bench.php"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
PHP_FPM_CONF="${SCRIPT_DIR}/php-fpm.conf"
NGINX_CONF="${SCRIPT_DIR}/nginx.conf"
PHP_ROUTER_DOC_ROOT="${SCRIPT_DIR}"
PHP_ROUTER_BIN="${REPO_ROOT}/target/release/php-router"
PHP_NATIVE_BIN="${REPO_ROOT}/target/release/php"

AUTO=1
START_PHP_ROUTER=1
START_NGINX=1

STARTED_PHP_ROUTER=0
STARTED_PHPFPM=0
STARTED_NGINX=0
PHP_ROUTER_PID=""
PHPFPM_PID=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    -n)
      N="$2"
      shift 2
      ;;
    -c)
      C="$2"
      shift 2
      ;;
    --ms)
      MS="$2"
      shift 2
      ;;
    --auto)
      AUTO=1
      shift
      ;;
    --start-php-router)
      START_PHP_ROUTER=1
      shift
      ;;
    --start-nginx)
      START_NGINX=1
      shift
      ;;
    --php-router-bin)
      PHP_ROUTER_BIN="$2"
      shift 2
      ;;
    --php-native-bin)
      PHP_NATIVE_BIN="$2"
      shift 2
      ;;
    --php-router)
      PHP_ROUTER_URL="$2"
      shift 2
      ;;
    --nginx)
      NGINX_URL="$2"
      shift 2
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

if [[ $AUTO -eq 1 ]]; then
  START_PHP_ROUTER=1
  START_NGINX=1
fi

port_listening() {
  local port="$1"
  if command -v lsof >/dev/null 2>&1; then
    lsof -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1
    return $?
  fi
  return 1
}

start_php_router() {
  if port_listening 8541; then
    return
  fi
  if [[ ! -x "$PHP_ROUTER_BIN" || ! -x "$PHP_NATIVE_BIN" ]]; then
    echo "php-router binaries not found; building..." >&2
    if ! (cd "$REPO_ROOT" && cargo build --release --features router --bin php-router --bin php); then
      echo "Failed to build php-router binaries. Try:" >&2
      echo "  (cd \"$REPO_ROOT\" && cargo build --release --features router --bin php-router --bin php)" >&2
      return 1
    fi
  fi
  PHP_NATIVE_BIN="$PHP_NATIVE_BIN" PHP_ROUTER_DOC_ROOT="$PHP_ROUTER_DOC_ROOT" "$PHP_ROUTER_BIN" \
    > "${SCRIPT_DIR}/php-router.log" 2>&1 &
  PHP_ROUTER_PID=$!
  STARTED_PHP_ROUTER=1
  sleep 0.4
  if ! kill -0 "$PHP_ROUTER_PID" >/dev/null 2>&1; then
    echo "php-router failed to start. See ${SCRIPT_DIR}/php-router.log" >&2
    return 1
  fi
}

start_php_fpm() {
  if port_listening 9000; then
    return
  fi
  php-fpm -y "$PHP_FPM_CONF" > "${SCRIPT_DIR}/php-fpm.out" 2>&1 &
  PHPFPM_PID=$!
  STARTED_PHPFPM=1
  sleep 0.4
  if ! kill -0 "$PHPFPM_PID" >/dev/null 2>&1; then
    echo "php-fpm failed to start. See ${SCRIPT_DIR}/php-fpm.out" >&2
    return 1
  fi
}

start_nginx() {
  if port_listening 8542; then
    return
  fi
  nginx -c "$NGINX_CONF" > "${SCRIPT_DIR}/nginx.out" 2>&1 || return 1
  STARTED_NGINX=1
  sleep 0.4
}

stop_servers() {
  if [[ $STARTED_NGINX -eq 1 ]]; then
    nginx -s stop -c "$NGINX_CONF" >/dev/null 2>&1 || true
  fi
  if [[ $STARTED_PHPFPM -eq 1 && -n "${PHPFPM_PID}" ]]; then
    kill "$PHPFPM_PID" >/dev/null 2>&1 || true
  fi
  if [[ $STARTED_PHP_ROUTER -eq 1 && -n "${PHP_ROUTER_PID}" ]]; then
    kill "$PHP_ROUTER_PID" >/dev/null 2>&1 || true
  fi
}

if [[ $START_PHP_ROUTER -eq 1 ]]; then
  start_php_router
fi

if [[ $START_NGINX -eq 1 ]]; then
  start_php_fpm
  start_nginx
fi

if [[ $AUTO -eq 1 || $START_PHP_ROUTER -eq 1 || $START_NGINX -eq 1 ]]; then
  trap stop_servers EXIT
fi

run_ab() {
  local label="$1"
  local url="$2"
  local out
  set +e
  out=$(ab -n "$N" -c "$C" "${url}?ms=${MS}" 2>&1)
  local status=$?
  set -e
  if [[ $status -ne 0 ]]; then
    printf "%s\n" "${label}"
    printf "  ab failed (exit %s)\n" "$status"
    printf "%s\n" "$out"
    return
  fi

  local rps
  local tpr
  local tpr_all
  local xfer
  local complete
  local failed

  rps=$(echo "$out" | awk -F': ' '/Requests per second/ {print $2}')
  tpr=$(echo "$out" | awk -F': ' '/Time per request/ && /mean\)/ {print $2; exit}')
  tpr_all=$(echo "$out" | awk -F': ' '/Time per request/ && /across all concurrent/ {print $2; exit}')
  xfer=$(echo "$out" | awk -F': ' '/Transfer rate/ {print $2}')
  complete=$(echo "$out" | awk -F': ' '/Complete requests/ {print $2}')
  failed=$(echo "$out" | awk -F': ' '/Failed requests/ {print $2}')

  printf "%s\n" "${label}"
  printf "  Requests per second: %s\n" "${rps:-n/a}"
  printf "  Time per request: %s\n" "${tpr:-n/a}"
  printf "  Time per request (all concurrent): %s\n" "${tpr_all:-n/a}"
  printf "  Transfer rate: %s\n" "${xfer:-n/a}"
  printf "  Complete requests: %s\n" "${complete:-n/a}"
  printf "  Failed requests: %s\n" "${failed:-n/a}"
}

echo "ab -n ${N} -c ${C} (ms=${MS})"
run_ab "php-router" "$PHP_ROUTER_URL"
echo ""
run_ab "nginx+php-fpm" "$NGINX_URL"

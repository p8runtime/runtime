#!/usr/bin/env bash
set -euo pipefail

PHP_BIN=${PHP_BIN:-php}
GILD_BIN=${GILD_NATIVE_BIN:-target/release/php}

if [ ! -x "$PHP_BIN" ]; then
  echo "official PHP binary not found at $PHP_BIN" >&2
  exit 1
fi

if [ ! -x "$GILD_BIN" ]; then
  GILD_BIN=target/debug/php
fi

if [ ! -x "$GILD_BIN" ]; then
  echo "gild native PHP binary not found; run \"cargo build --bin php\" or set GILD_NATIVE_BIN" >&2
  exit 1
fi

SCRIPT=${1:-test-001/hello.php}
shift || true

if [ ! -f "$SCRIPT" ]; then
  echo "script not found: $SCRIPT" >&2
  exit 1
fi

OUTDIR=$(mktemp -d)
trap 'rm -rf "$OUTDIR"' EXIT

run_script() {
  local label=$1
  local bin=$2
  local out="$OUTDIR/${label}.txt"
  "$bin" "$SCRIPT" "$@" >"$out" 2>&1
  echo "$out"
}

official_out=$(run_script official "$PHP_BIN" "$@")
gild_out=$(run_script gild "$GILD_BIN" "$@")

echo "=== Official PHP ($PHP_BIN) ==="
cat "$official_out"
echo
echo "=== Gild executable ($GILD_BIN) ==="
cat "$gild_out"
echo

if diff -u "$official_out" "$gild_out"; then
  echo "Outputs match." >&2
else
  echo "Outputs differ: compare $official_out vs $gild_out" >&2
  exit 1
fi

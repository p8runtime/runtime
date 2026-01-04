#!/usr/bin/env bash
set -euo pipefail

wasmtime target/wasm32-wasip1/debug/gild-wasm.wasm -- test-001/hello.php

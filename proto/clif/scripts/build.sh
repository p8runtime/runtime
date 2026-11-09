#!/bin/sh
# Build #397's prototype: the generator, the object it writes, the harness
# that links it. Run from proto/clif. [triple] defaults to x86-64 Linux.
#
#   scripts/build.sh [triple]
set -eu
TRIPLE=${1:-x86_64-unknown-linux-gnu}
cargo build --release --locked --bin clif-gen
"${CARGO_TARGET_DIR:-target}/release/clif-gen" out "$TRIPLE"
cargo build --release --locked --bin clif
echo BUILD-DONE

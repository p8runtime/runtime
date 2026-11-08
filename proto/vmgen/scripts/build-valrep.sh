#!/bin/sh
# #391 on addis: build the prototype from ~/p8-perf/valrep/src under the
# build lock, keep it as bin/vmgen-<name>, and disassemble the copying
# handlers into disasm-<name>.txt.   build-valrep.sh <name>
set -u
D=$HOME/p8-perf/valrep
cd "$D"
flock ~/p8-perf/.build.lock nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc -c env CARGO_HOME=$D/cargo-home \
  CARGO_TARGET_DIR=$D/target TMPDIR=$D/tmp \
  cargo build --release --locked --manifest-path src/proto/vmgen/Cargo.toml > build-$1.log 2>&1 || { echo "build failed: $1"; tail -20 build-$1.log; exit 1; }
cp target/release/vmgen bin/vmgen-$1
TMPDIR=$D/tmp nix shell nixpkgs#binutils -c sh src/proto/vmgen/scripts/disasm.sh bin/vmgen-$1 \
  P8_ASSIGN_CONST_COPY P8_ASSIGN_CV_COPY P8_ASSIGN_TMP_COPY P8_ASSIGN_TMP_TAKE P8_MOVE_TMP_CV P8_MOVE_TMP_CONST > disasm-$1.txt 2>&1
echo "built $1"

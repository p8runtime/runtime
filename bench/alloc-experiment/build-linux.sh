#!/bin/sh
# Runs on addis from ~/p8-lane-alloc. Builds php2 once per allocator from the
# same source tree (src/ = git archive of claude/alloc-experiment), each build
# under the shared heavy-job lock; keeps each binary as php2-<name>.
cd ~/p8-lane-alloc || exit 2
export CARGO_HOME=$HOME/p8-lane-alloc/cargo-home CARGO_TARGET_DIR=$HOME/p8-lane-alloc/target
for pair in glibc: dlmalloc:alloc-dlmalloc rlsf:alloc-rlsf talc:alloc-talc rusty:alloc-rusty; do
  name=${pair%%:*} feat=${pair#*:}
  flags=""
  [ -n "$feat" ] && flags="--features $feat"
  start=$(date +%s)
  flock ~/p8-perf/.build.lock nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc -c env CARGO_PROFILE_RELEASE_DEBUG=line-tables-only cargo build --release --locked -p php-cli $flags --manifest-path src/Cargo.toml > build-$name.log 2>&1
  rc=$?
  if [ $rc -eq 0 ]; then cp target/release/php2 php2-$name; fi
  echo "built $name rc=$rc $(( $(date +%s) - start ))s"
done

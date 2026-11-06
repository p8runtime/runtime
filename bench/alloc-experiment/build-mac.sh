#!/bin/sh
# Runs on simien from ~/p8-lane-alloc. Builds php2 once per allocator crate
# from the same source tree (src/ = git archive of claude/alloc-experiment),
# one build at a time; keeps each binary as php2-<name>. php2-system (no
# feature) is built first by hand.
cd ~/p8-lane-alloc || exit 2
export CARGO_TARGET_DIR=$HOME/p8-lane-alloc/target
for name in dlmalloc rlsf talc rusty; do
  start=$(date +%s)
  cargo build --release --locked -p php-cli --features alloc-$name --manifest-path src/Cargo.toml > build-$name.log 2>&1
  rc=$?
  if [ $rc -eq 0 ]; then cp target/release/php2 php2-$name; fi
  echo "built $name rc=$rc $(( $(date +%s) - start ))s"
done
echo all-done

#!/usr/bin/env bash
# One case under callgrind with per-instruction and per-line costs, at
# bench/count's two iteration counts, for lines.py to subtract.
#
# Usage: bench/calls/prof.sh <out dir> <label> <binary> <case.php>
# Writes <out dir>/<label>.<case>.<n>.cg for n = 2000 and 22000.
set -euo pipefail
out=$1 label=$2 bin=$3 file=$4
name=$(basename "$file" .php)
mkdir -p "$out"
for n in 2000 22000; do
    valgrind --tool=callgrind --dump-instr=yes --dump-line=yes --compress-pos=no --compress-strings=no --callgrind-out-file="$out/$label.$name.$n.cg" "$bin" "$file" "$n" >/dev/null 2>"$out/$label.$name.$n.log"
done
grep -h 'Collected' "$out/$label.$name".*.log

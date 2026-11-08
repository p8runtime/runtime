#!/bin/sh
# #391: measure.sh with a fourth binary and the store-forward count.
# Instructions per loop iteration (callgrind at N and 2N, difference / N),
# cycles and ld_blocks.store_forward per iteration (perf stat, the same
# way at M and 2M, mean of 5), and the outputs of the check variants
# against PHP 8.6. Runs inside a nix shell with valgrind and perf, under
# the build lock.
#
#   measure-valrep.sh <php86> <php2-main> <vmgen-v1> <vmgen-new> <outdir> [micro...]
set -u
PHP=$1; P8=$2; V1=$3; VM=$4; OUT=$5; shift 5
cd "$(dirname "$0")/.."
mkdir -p "$OUT"
MICROS=${*:-"empty add assign call method prop_read prop_write prop_typed arr_read arr_write isset_dim concat new identical"}
N=20000
CG="valgrind --tool=callgrind --dump-instr=yes --compress-pos=no --compress-strings=no"
ir() { grep "I   refs" "$1" | grep -o "[0-9,]*$" | tr -d ,; }
bin() { case $1 in php) echo "$PHP";; p8) echo "$P8";; v1) echo "$V1";; vm) echo "$VM";; esac; }

echo "== outputs (check variants, n=7) against PHP 8.6" > "$OUT/outputs.txt"
for m in $MICROS; do
  f=micro/check/$m.php
  "$PHP" -n -d display_errors=1 "$f" 7 > "$OUT/out-php-$m.txt" 2>&1
  line=$(printf "%-11s php=%-14s" "$m" "$(tr -d '\n' < "$OUT/out-php-$m.txt" | cut -c1-14)")
  for e in p8 v1 vm; do
    "$(bin $e)" "$f" 7 > "$OUT/out-$e-$m.txt" 2>&1; rc=$?
    s=same; cmp -s "$OUT/out-php-$m.txt" "$OUT/out-$e-$m.txt" || s=DIFFERENT
    [ $rc -eq 2 ] && s=refused
    line="$line $e:$s"
  done
  echo "$line" >> "$OUT/outputs.txt"
done
cat "$OUT/outputs.txt"

echo "== instructions per iteration (callgrind, (I(2N) - I(N)) / N, N=$N)" > "$OUT/instructions.txt"
printf "%-11s %8s %8s %8s %8s\n" program php86 p8-main V1 valrep >> "$OUT/instructions.txt"
for m in $MICROS; do
  line=$(printf "%-11s" "$m")
  for e in php p8 v1 vm; do
    B=$(bin $e); n2=$((N * 2))
    if ! $CG --callgrind-out-file="$OUT/cg-$e-$m-$N.out" "$B" micro/$m.php $N > /dev/null 2> "$OUT/cg-$e-$m-$N.log"; then
      line="$line $(printf '%8s' refused)"; rm -f "$OUT/cg-$e-$m-$N.out"; continue
    fi
    $CG --callgrind-out-file="$OUT/cg-$e-$m-$n2.out" "$B" micro/$m.php $n2 > /dev/null 2> "$OUT/cg-$e-$m-$n2.log"
    a=$(ir "$OUT/cg-$e-$m-$N.log"); b=$(ir "$OUT/cg-$e-$m-$n2.log")
    line="$line $(printf '%8s' $(( (b - a) / N )))"
    rm -f "$OUT/cg-$e-$m-$N.out"
    # (Only the experiment's per-address dump is kept, for the breakdown.)
    [ $e = vm ] || rm -f "$OUT/cg-$e-$m-$n2.out"
  done
  echo "$line" >> "$OUT/instructions.txt"
done
cat "$OUT/instructions.txt"

M=5000000
echo "== per iteration (perf stat :u, (x(2M) - x(M)) / M, M=$M, mean of 5): cycles, then ld_blocks.store_forward x100" > "$OUT/cycles.txt"
printf "%-11s %8s %8s %8s %8s | %8s %8s %8s %8s\n" program php86 p8-main V1 valrep php86 p8-main V1 valrep >> "$OUT/cycles.txt"
st() { # binary file n -> "cycles stores" (mean of 5)
  tc=0; ts=0
  for i in 1 2 3 4 5; do
    perf stat -x, -e cycles:u,ld_blocks.store_forward:u -o "$OUT/perf.tmp" -- "$1" "$2" "$3" > /dev/null 2>&1
    c=$(grep ',cycles' "$OUT/perf.tmp" | cut -d, -f1); s=$(grep 'store_forward' "$OUT/perf.tmp" | cut -d, -f1)
    tc=$((tc + c)); ts=$((ts + s))
  done
  echo "$((tc / 5)) $((ts / 5))"
}
for m in $MICROS; do
  cy=$(printf "%-11s" "$m"); sf=""
  for e in php p8 v1 vm; do
    B=$(bin $e)
    if ! "$B" micro/$m.php 1 > /dev/null 2>&1; then cy="$cy $(printf '%8s' refused)"; sf="$sf $(printf '%8s' -)"; continue; fi
    set -- $(st "$B" micro/$m.php $M); c1=$1; s1=$2
    set -- $(st "$B" micro/$m.php $((M * 2))); c2=$1; s2=$2
    cy="$cy $(printf '%8s' $(( (c2 - c1) / M )))"
    sf="$sf $(printf '%8s' $(( (s2 - s1) * 100 / M )))"
  done
  echo "$cy |$sf" >> "$OUT/cycles.txt"
done
rm -f "$OUT/perf.tmp"
cat "$OUT/cycles.txt"
echo MEASURE-DONE

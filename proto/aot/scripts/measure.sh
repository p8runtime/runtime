#!/bin/sh
# V1's measurement (proto/vmgen/scripts/measure.sh) with the two AOT
# flavours beside it: machine instructions per loop iteration (callgrind at
# N and 2N, difference / N), cycles per iteration (perf stat, the same way,
# at a larger M), and the outputs of the check variants against PHP 8.6.
# Runs inside the nix shell that has valgrind and perf, under the build lock.
#
#   measure.sh <php86> <php2-main> <vmgen> <aot-a> <aot-b> <outdir>
#
# Each binary is given <file> <n>. Run from proto/vmgen (its micro/).
set -u
PHP=$1; P8=$2; VM=$3; AA=$4; AB=$5; OUT=$6
mkdir -p "$OUT"
MICROS="empty add assign call method prop_read prop_write prop_typed arr_read arr_write isset_dim concat new identical"
ENGINES="php p8 vm a b"
N=20000
CG="valgrind --tool=callgrind --dump-instr=yes --compress-pos=no --compress-strings=no"
ir() { grep "I   refs" "$1" | grep -o "[0-9,]*$" | tr -d ,; }
bin() { case $1 in php) echo "$PHP";; p8) echo "$P8";; vm) echo "$VM";; a) echo "$AA";; b) echo "$AB";; esac; }

echo "== outputs (check variants, n=7 and n=0): each engine vs PHP 8.6" > "$OUT/outputs.txt"
for m in $MICROS; do
  for n in 7 0; do
    f=micro/check/$m.php
    "$PHP" -n -d display_errors=1 "$f" $n > "$OUT/out-php-$m-$n.txt" 2>&1
    line=$(printf "%-11s n=%s php=%-8s" "$m" "$n" "$(tr -d '\n' < "$OUT/out-php-$m-$n.txt" | cut -c1-8)")
    for e in p8 vm a b; do
      "$(bin $e)" "$f" $n > "$OUT/out-$e-$m-$n.txt" 2>&1; rc=$?
      r=same; cmp -s "$OUT/out-php-$m-$n.txt" "$OUT/out-$e-$m-$n.txt" || r=DIFFERENT
      line="$line $e:$r($rc)"
    done
    echo "$line" >> "$OUT/outputs.txt"
  done
done
cat "$OUT/outputs.txt"

echo "== instructions per iteration (callgrind, (I(2N) - I(N)) / N, N=$N)" > "$OUT/instructions.txt"
printf "%-11s %8s %8s %8s %8s %8s\n" program php86 p8-main V1 V4a V4b >> "$OUT/instructions.txt"
for m in $MICROS; do
  line=$(printf "%-11s" "$m")
  for e in $ENGINES; do
    B=$(bin $e); n2=$((N * 2))
    $CG --callgrind-out-file="$OUT/cg-$e-$m-$N.out" "$B" micro/$m.php $N > /dev/null 2> "$OUT/cg-$e-$m-$N.log" || { line="$line $(printf '%8s' fail)"; continue; }
    $CG --callgrind-out-file="$OUT/cg-$e-$m-$n2.out" "$B" micro/$m.php $n2 > /dev/null 2> "$OUT/cg-$e-$m-$n2.log"
    a=$(ir "$OUT/cg-$e-$m-$N.log"); b=$(ir "$OUT/cg-$e-$m-$n2.log")
    line="$line $(printf '%8s' $(( (b - a) / N )))"
    rm -f "$OUT/cg-$e-$m-$N.out"
    # (The 2N per-address dump is kept for V4b's attribution only.)
    [ "$e" = b ] || rm -f "$OUT/cg-$e-$m-$n2.out"
  done
  echo "$line" >> "$OUT/instructions.txt"
done
cat "$OUT/instructions.txt"

M=5000000
echo "== cycles per iteration (perf stat cycles:u, (c(2M) - c(M)) / M, M=$M, mean of 5)" > "$OUT/cycles.txt"
printf "%-11s %8s %8s %8s %8s %8s\n" program php86 p8-main V1 V4a V4b >> "$OUT/cycles.txt"
cyc() { # binary file n: mean cycles:u of 5 runs
  t=0
  for i in 1 2 3 4 5; do
    c=$(perf stat -x, -e cycles:u -- "$1" "$2" "$3" 2>&1 >/dev/null | grep cycles | cut -d, -f1)
    t=$((t + c))
  done
  echo $((t / 5))
}
for m in $MICROS; do
  line=$(printf "%-11s" "$m")
  for e in $ENGINES; do
    B=$(bin $e)
    a=$(cyc "$B" micro/$m.php $M); b=$(cyc "$B" micro/$m.php $((M * 2)))
    line="$line $(printf '%8s' $(( (b - a) / M )))"
  done
  echo "$line" >> "$OUT/cycles.txt"
done
cat "$OUT/cycles.txt"
echo MEASURE-DONE

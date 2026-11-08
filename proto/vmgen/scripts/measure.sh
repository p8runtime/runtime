#!/bin/sh
# Measure the micro programs on addis: machine instructions per loop
# iteration (callgrind at N and 2N, difference / N), cycles per iteration
# (perf stat, the same way, at a larger N), and the outputs of the check
# variants against PHP 8.6. Runs inside the nix shell that has valgrind and
# perf, under the build lock.
#
#   measure.sh <php86> <php2-main> <vmgen> <outdir>
#
# Each binary is given <file> <n>: PHP runs the file (its main calls t);
# the prototype runs t(n) directly, after compiling the file.
set -u
PHP=$1; P8=$2; VM=$3; OUT=$4
cd "$(dirname "$0")/.."
mkdir -p "$OUT"
MICROS="empty add assign call call_typed method prop_read prop_write prop_typed arr_read arr_write isset_dim builtin strlen concat new identical"
N=20000
CG="valgrind --tool=callgrind --dump-instr=yes --compress-pos=no --compress-strings=no"
ir() { grep "I   refs" "$1" | grep -o "[0-9,]*$" | tr -d ,; }

echo "== outputs (check variants, n=7): PHP 8.6 vs p8 main vs prototype" > "$OUT/outputs.txt"
for m in $MICROS; do
  f=micro/check/$m.php
  "$PHP" -n -d display_errors=1 "$f" 7 > "$OUT/out-php-$m.txt" 2>&1; a=$?
  "$P8" "$f" 7 > "$OUT/out-p8-$m.txt" 2>&1; b=$?
  "$VM" "$f" 7 > "$OUT/out-vm-$m.txt" 2>&1; c=$?
  vp=same; cmp -s "$OUT/out-php-$m.txt" "$OUT/out-vm-$m.txt" || vp=DIFFERENT
  p8p=same; cmp -s "$OUT/out-php-$m.txt" "$OUT/out-p8-$m.txt" || p8p=DIFFERENT
  [ $c -eq 2 ] && vp="refused: $(head -1 "$OUT/out-vm-$m.txt")"
  printf "%-11s php=%s p8=%s vm=%s  p8:%s  vm:%s\n" "$m" "$(tr -d '\n' < "$OUT/out-php-$m.txt" | cut -c1-40)" "$b" "$c" "$p8p" "$vp" >> "$OUT/outputs.txt"
done
cat "$OUT/outputs.txt"

echo "== instructions per iteration (callgrind, (I(2N) - I(N)) / N, N=$N)" > "$OUT/instructions.txt"
printf "%-11s %10s %10s %10s\n" program php86 p8-main vmgen >> "$OUT/instructions.txt"
for m in $MICROS; do
  line=$(printf "%-11s" "$m")
  for e in php p8 vm; do
    case $e in php) B=$PHP;; p8) B=$P8;; vm) B=$VM;; esac
    n2=$((N * 2))
    $CG --callgrind-out-file="$OUT/cg-$e-$m-$N.out" "$B" micro/$m.php $N > /dev/null 2> "$OUT/cg-$e-$m-$N.log"
    rc=$?
    if [ $rc -ne 0 ]; then line="$line $(printf '%10s' refused)"; rm -f "$OUT/cg-$e-$m-$N.out"; continue; fi
    $CG --callgrind-out-file="$OUT/cg-$e-$m-$n2.out" "$B" micro/$m.php $n2 > /dev/null 2> "$OUT/cg-$e-$m-$n2.log"
    a=$(ir "$OUT/cg-$e-$m-$N.log"); b=$(ir "$OUT/cg-$e-$m-$n2.log")
    line="$line $(printf '%10s' $(( (b - a) / N )))"
    # (The per-address dump at 2N is kept for the handler-level breakdown; the N one goes.)
    rm -f "$OUT/cg-$e-$m-$N.out"
  done
  echo "$line" >> "$OUT/instructions.txt"
done
cat "$OUT/instructions.txt"

M=5000000
echo "== cycles per iteration (perf stat cycles:u, (c(2M) - c(M)) / M, M=$M, mean of 5)" > "$OUT/cycles.txt"
printf "%-11s %10s %10s %10s\n" program php86 p8-main vmgen >> "$OUT/cycles.txt"
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
  for e in php p8 vm; do
    case $e in php) B=$PHP;; p8) B=$P8;; vm) B=$VM;; esac
    if ! "$B" micro/$m.php 1 > /dev/null 2>&1; then line="$line $(printf '%10s' refused)"; continue; fi
    a=$(cyc "$B" micro/$m.php $M); b=$(cyc "$B" micro/$m.php $((M * 2)))
    line="$line $(printf '%10s' $(( (b - a) / M )))"
  done
  echo "$line" >> "$OUT/cycles.txt"
done
cat "$OUT/cycles.txt"
echo MEASURE-DONE

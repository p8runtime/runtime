#!/bin/sh
# V4's measurement (proto/aot/scripts/measure.sh) for #397's three
# programs, Cranelift beside the others: outputs of the check variants
# against PHP 8.6 (n=7, n=0), instructions per iteration (callgrind,
# (I(2N) - I(N)) / N), cycles per iteration (perf stat cycles:u,
# (c(2M) - c(M)) / M, mean of 5). Inside a nix shell with valgrind and
# perf, under the build lock. Run from proto/vmgen (its micro/).
#
#   measure.sh <php86> <V1 vmgen> <V4b aot-b> <clif> <outdir>
set -u
PHP=$1; VM=$2; AB=$3; CL=$4; OUT=$5
mkdir -p "$OUT"
MICROS="add call prop_read"
ENGINES="php vm b clif"
N=20000
CG="valgrind --tool=callgrind --dump-instr=yes --compress-pos=no --compress-strings=no"
ir() { grep "I   refs" "$1" | grep -o "[0-9,]*$" | tr -d ,; }
bin() { case $1 in php) echo "$PHP -n";; vm) echo "$VM";; b) echo "$AB";; clif) echo "$CL";; esac; }

echo "== outputs (check variants, n=7 and n=0): each engine vs PHP 8.6" > "$OUT/outputs.txt"
for m in $MICROS; do
  for n in 7 0; do
    f=micro/check/$m.php
    $PHP -n -d display_errors=1 "$f" $n > "$OUT/out-php-$m-$n.txt" 2>&1
    line=$(printf "%-11s n=%s php=%-8s" "$m" "$n" "$(tr -d '\n' < "$OUT/out-php-$m-$n.txt" | cut -c1-8)")
    for e in vm b clif; do
      $(bin $e) "$f" $n > "$OUT/out-$e-$m-$n.txt" 2>&1; rc=$?
      r=same; cmp -s "$OUT/out-php-$m-$n.txt" "$OUT/out-$e-$m-$n.txt" || r=DIFFERENT
      line="$line $e:$r($rc)"
    done
    echo "$line" >> "$OUT/outputs.txt"
  done
done
cat "$OUT/outputs.txt"

echo "== instructions per iteration (callgrind, (I(2N) - I(N)) / N, N=$N)" > "$OUT/instructions.txt"
printf "%-11s %8s %8s %8s %8s\n" program php86 V1 V4b clif >> "$OUT/instructions.txt"
for m in $MICROS; do
  line=$(printf "%-11s" "$m")
  for e in $ENGINES; do
    n2=$((N * 2))
    $CG --callgrind-out-file="$OUT/cg-$e-$m-$N.out" $(bin $e) micro/$m.php $N > /dev/null 2> "$OUT/cg-$e-$m-$N.log" || { line="$line $(printf '%8s' fail)"; continue; }
    $CG --callgrind-out-file="$OUT/cg-$e-$m-$n2.out" $(bin $e) micro/$m.php $n2 > /dev/null 2> "$OUT/cg-$e-$m-$n2.log"
    a=$(ir "$OUT/cg-$e-$m-$N.log"); b=$(ir "$OUT/cg-$e-$m-$n2.log")
    line="$line $(printf '%8s' $(( (b - a) / N )))"
    rm -f "$OUT/cg-$e-$m-$N.out"
    # (The 2N per-address dumps of V4b and clif are kept for the comparison.)
    case $e in b|clif) ;; *) rm -f "$OUT/cg-$e-$m-$n2.out";; esac
  done
  echo "$line" >> "$OUT/instructions.txt"
done
cat "$OUT/instructions.txt"

M=5000000
echo "== cycles per iteration (perf stat cycles:u, (c(2M) - c(M)) / M, M=$M, mean of 5)" > "$OUT/cycles.txt"
printf "%-11s %8s %8s %8s %8s\n" program php86 V1 V4b clif >> "$OUT/cycles.txt"
cyc() { # engine file n: mean cycles:u of 5 runs
  t=0
  for i in 1 2 3 4 5; do
    c=$(perf stat -x, -e cycles:u -- $(bin $1) "$2" "$3" 2>&1 >/dev/null | grep cycles | cut -d, -f1)
    t=$((t + c))
  done
  echo $((t / 5))
}
for m in $MICROS; do
  line=$(printf "%-11s" "$m")
  for e in $ENGINES; do
    a=$(cyc $e micro/$m.php $M); b=$(cyc $e micro/$m.php $((M * 2)))
    line="$line $(printf '%8s' $(( (b - a) / M )))"
  done
  echo "$line" >> "$OUT/cycles.txt"
done
cat "$OUT/cycles.txt"
echo MEASURE-DONE

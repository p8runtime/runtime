#!/bin/sh
# Where V4b's instructions per iteration go, by the source file of the
# code that ran (callgrind attributes inlined code to the file it was
# inlined from): callgrind at N and 2N, per-file self cost, difference / N.
# Runs in the nix shell with valgrind, from proto/vmgen.
#
#   attrib.sh <aot-b> <outdir> [programs...]
set -u
B=$1; OUT=$2; shift 2
MICROS=${*:-"empty add assign call method prop_read prop_write prop_typed arr_read arr_write isset_dim concat new identical"}
N=20000
mkdir -p "$OUT"
files() { # callgrind out → "<Ir>\t<file>" summed per file
  callgrind_annotate --auto=no --inclusive=no --threshold=100 "$1" 2>/dev/null \
    | grep -E '^ *[0-9,]+ ' \
    | sed -E 's/^ *([0-9,]+) (\([^)]*\) +)?([^:]*):.*/\1\t\3/' | tr -d , \
    | awk -F'\t' '{ s[$2] += $1 } END { for (f in s) print s[f] "\t" f }'
}
for m in $MICROS; do
  for n in $N $((N * 2)); do
    valgrind --tool=callgrind --callgrind-out-file="$OUT/cgb-$m-$n.out" "$B" micro/$m.php $n > /dev/null 2>&1
    files "$OUT/cgb-$m-$n.out" > "$OUT/files-$m-$n.tsv"
    rm -f "$OUT/cgb-$m-$n.out"
  done
  echo "== $m: instructions per iteration by source file"
  awk -F'\t' -v N=$N 'FNR == NR { a[$2] = $1; next } { d = ($1 - a[$2]) / N; if (d >= 0.5 || d <= -0.5) printf "%7.1f  %s\n", d, $2 }' \
    "$OUT/files-$m-$N.tsv" "$OUT/files-$m-$((N * 2)).tsv" | sort -rn
done
echo ATTRIB-DONE

#!/usr/bin/env bash
# First execution of a call site against its second (gen.sh's first/*).
#
# Usage: bench/calls/first.sh <dir gen.sh wrote> label=binary [label=binary ...]
# Needs valgrind. Prints, per engine and kind, machine instructions per
# call site: the first run of run() and the second.
set -euo pipefail
dir=${1:?usage: first.sh <dir> label=binary ...}
shift
M=$(cat "$dir/first/M")
count() {
    valgrind --tool=callgrind --callgrind-out-file=/dev/null "$1" "$2" 2>&1 >/dev/null | sed -n 's/.*Collected : \([0-9]*\).*/\1/p'
}
printf '%-10s %-8s %12s %12s %12s %10s %10s\n' engine kind never once twice first second
for e in "$@"; do
    label=${e%%=*} bin=${e#*=}
    for kind in plain fn method static; do
        c0=$(count "$bin" "$dir/first/$kind.0.php")
        c1=$(count "$bin" "$dir/first/$kind.1.php")
        c2=$(count "$bin" "$dir/first/$kind.2.php")
        printf '%-10s %-8s %12s %12s %12s %10s %10s\n' "$label" "$kind" "$c0" "$c1" "$c2" $(((c1 - c0) / M)) $(((c2 - c1) / M))
    done
done

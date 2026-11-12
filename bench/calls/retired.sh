#!/usr/bin/env bash
# Instructions retired and cycles per loop iteration on macOS (no valgrind
# on arm64 macOS): bench/count's method, with the kernel's counters that
# `/usr/bin/time -l` prints in place of callgrind's count.
#
# Usage: bench/calls/retired.sh <cases dir> label=binary [label=binary ...]
#   RETIRED_CASES=<glob of case names>   (default: all)
#   RETIRED_RUNS=<n>                     runs per point, the lowest is kept (default 5)
#
# Each case runs at LOW and HIGH iterations; (count(HIGH) - count(LOW)) /
# (HIGH - LOW) is one iteration. Instructions retired vary by a few
# thousand between runs (the kernel's share of the process), which over
# 2,000,000 iterations is under 0.01 of an instruction; cycles vary with
# what else the machine is doing, so they are the lowest of the runs and
# still only as good as the machine was quiet.
# Engines are run as bench/count runs them: `<binary> <case> <iterations>`.
# Set TMPDIR to a folder of your own first (mktemp).
set -euo pipefail
dir=${1:?usage: retired.sh <cases dir> label=binary ...}
shift
LOW=1000000
HIGH=3000000
runs=${RETIRED_RUNS:-5}
tmp=$(mktemp -d)
trap 'find "$tmp" -mindepth 1 -delete; rmdir "$tmp"' EXIT

# lowest "<instructions> <cycles>" of $runs runs; the program's output is left in $tmp/out.<n>
point() {
    local bin=$1 file=$2 n=$3 best_i="" best_c="" i c r
    for ((r = 0; r < runs; r++)); do
        /usr/bin/time -l "$bin" "$file" "$n" >"$tmp/out" 2>"$tmp/time" || { echo "FAILED FAILED"; return; }
        i=$(awk '/instructions retired/ {print $1}' "$tmp/time")
        c=$(awk '/cycles elapsed/ {print $1}' "$tmp/time")
        if [ -z "$best_i" ] || [ "$i" -lt "$best_i" ]; then best_i=$i; fi
        if [ -z "$best_c" ] || [ "$c" -lt "$best_c" ]; then best_c=$c; fi
    done
    echo "$best_i $best_c"
}

printf '%-26s' case
for e in "$@"; do printf ' %12s %10s' "${e%%=*} instr" cycles; done
printf '\n'
for f in "$dir"/${RETIRED_CASES:-*}.php; do
    name=$(basename "$f" .php)
    printf '%-26s' "$name"
    want=""
    for e in "$@"; do
        bin=${e#*=}
        read -r li lc <<<"$(point "$bin" "$f" $LOW)"
        read -r hi hc <<<"$(point "$bin" "$f" $HIGH)"
        got=$(cksum <"$tmp/out")
        if [ "$li" = FAILED ] || [ "$hi" = FAILED ]; then
            printf ' %12s %10s' FAILED ""
        elif [ -n "$want" ] && [ "$got" != "$want" ]; then
            printf ' %12s %10s' WRONG ""
        else
            printf ' %12s %10s' "$(awk -v a="$hi" -v b="$li" -v n=$((HIGH - LOW)) 'BEGIN {printf "%.1f", (a - b) / n}')" \
                "$(awk -v a="$hc" -v b="$lc" -v n=$((HIGH - LOW)) 'BEGIN {printf "%.1f", (a - b) / n}')"
        fi
        [ -n "$want" ] || want=$got
    done
    printf '\n'
done

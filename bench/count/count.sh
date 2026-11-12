#!/usr/bin/env bash
# Machine instructions per loop iteration, for each case in cases/.
#
# Usage: bench/count/count.sh <engine> [<engine>...]
#   An engine is a path to a p8 or php binary; give it a label as label=path.
#   COUNT_JOBS=<n>   cases run at once (default: the number of CPUs)
#   COUNT_CASES=<glob of case names>   (default: all)
#
# Each case runs twice under valgrind's callgrind, at LOW and HIGH
# iterations; (instructions(HIGH) - instructions(LOW)) / (HIGH - LOW) is the
# cost of one iteration, with startup and compile time cancelled. The count
# is of instructions executed, not time: it is the same on every run and on
# a busy machine, so cases and lanes can run in parallel.
#
# Every engine's output must match the first engine's, case by case: a
# count from an engine that prints something else is reported as WRONG.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
LOW=2000
HIGH=22000
jobs=${COUNT_JOBS:-$(nproc)}
[ $# -ge 1 ] || { sed -n 2,8p "$0"; exit 2; }
command -v valgrind >/dev/null || { echo "valgrind is not installed" >&2; exit 2; }

labels=() paths=()
for e in "$@"; do
    case $e in
        *=*) labels+=("${e%%=*}"); paths+=("${e#*=}") ;;
        *) labels+=("$(basename "$e")"); paths+=("$e") ;;
    esac
done

out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

# One (engine, case): "<per-iteration count>" or "WRONG"/"FAILED" in $out/<i>.<case>.
one() {
    local i=$1 bin=$2 file=$3 name lo hi
    name=$(basename "$file" .php)
    count() {
        valgrind --tool=callgrind --callgrind-out-file=/dev/null "$bin" "$file" "$1" 2>&1 >"$out/$i.$name.$1.txt" \
            | sed -n 's/.*Collected : \([0-9]*\).*/\1/p'
    }
    lo=$(count $LOW) && hi=$(count $HIGH) || true
    if [ -z "${lo:-}" ] || [ -z "${hi:-}" ]; then
        echo FAILED >"$out/$i.$name"
    else
        echo $(((hi - lo + (HIGH - LOW) / 2) / (HIGH - LOW))) >"$out/$i.$name"
    fi
}
export -f one
export out LOW HIGH

cases=()
for f in "$here"/cases/${COUNT_CASES:-*}.php; do cases+=("$f"); done

for i in "${!paths[@]}"; do
    for f in "${cases[@]}"; do printf '%s\0%s\0%s\0' "$i" "${paths[$i]}" "$f"; done
done | xargs -0 -n 3 -P "$jobs" bash -c 'one "$@"' _

printf '%-18s' case
for l in "${labels[@]}"; do printf '%12s' "$l"; done
echo
status=0
for f in "${cases[@]}"; do
    name=$(basename "$f" .php)
    printf '%-18s' "$name"
    for i in "${!paths[@]}"; do
        v=$(cat "$out/$i.$name")
        if [ "$i" != 0 ] && ! cmp -s "$out/0.$name.$HIGH.txt" "$out/$i.$name.$HIGH.txt"; then v=WRONG; fi
        case $v in WRONG | FAILED) status=1 ;; esac
        printf '%12s' "$v"
    done
    echo
done
exit $status

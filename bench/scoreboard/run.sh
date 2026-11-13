#!/bin/sh
# bench/scoreboard/run.sh <sha> [host]
#
# Counts main's p8 binary for <sha> and PHP 8.6 on the measuring host's app
# kit (~/p8-perf/apps/count.sh: WordPress front page and the Laravel
# workload, machine instructions under callgrind), and appends one row to
# results.tsv beside this script. The host (default: addis) must already
# have the binary ~/p8-perf/calls/bin/main-<first 8 of sha>: this script
# does not build. Run from a checkout of the repository, on a machine that
# can `ssh <host>`. On any failure it says why, exits non-zero and appends
# nothing.
set -u

say() { printf '%s\n' "scoreboard: $*" >&2; }
fail() { say "$*"; exit 1; }

[ $# -ge 1 ] && [ $# -le 2 ] || { sed -n 2,11p "$0" >&2; exit 2; }
here=$(cd "$(dirname "$0")" && pwd) || exit 1
host=${2:-addis}
sha=$(git -C "$here" rev-parse --verify --quiet "$1^{commit}") || fail "$1 is not a commit in this checkout."
sha8=$(printf '%s' "$sha" | cut -c1-8)
day=$(date -u +%Y-%m-%d)
label="score-$(printf '%s' "$day" | tr -d -)-$sha8"
plabel="$label-php"
results="$here/results.tsv"

on_host() { ssh -o BatchMode=yes "$host" "$@"; }

# The binary: built before, by the calls kit's recipe (build.sh profiling).
on_host "test -x ~/p8-perf/calls/bin/main-$sha8" ||
    fail "There is no binary ~/p8-perf/calls/bin/main-$sha8 on $host (or $host cannot be reached). Build it first with ~/p8-perf/calls/build.sh profiling <source dir> main-$sha8; this script does not build."

# One count of <binary> under <label>, printed as count.sh's counts.txt. A
# label that was already counted (today, for this sha) is read back, not
# counted again: count.sh refuses a label twice.
counts() {
    on_host "set -u
        O=~/p8-perf/apps/out/$2
        if [ -e \"\$O/counts.txt\" ]; then
            echo \"scoreboard: $2 was counted before; reading its counts.txt\" >&2
        else
            timeout 3600 ~/p8-perf/apps/count.sh $1 $2 > /dev/null || exit 1
        fi
        cat \"\$O/counts.txt\""
}

# PHP is the kit's own (count.sh's PHP= line), counted in the same session.
php_bin=$(on_host "sed -n 's/^PHP=//p' ~/p8-perf/apps/count.sh") || fail "Could not read the PHP binary from $host's count.sh."
[ -n "$php_bin" ] || fail "$host's count.sh names no PHP binary (no PHP= line)."

p8_counts=$(counts "\$HOME/p8-perf/calls/bin/main-$sha8" "$label") || fail "Counting main-$sha8 as $label failed on $host; see ~/p8-perf/apps/out/$label/."
php_counts=$(counts "$php_bin" "$plabel") || fail "Counting PHP as $plabel failed on $host; see ~/p8-perf/apps/out/$plabel/."

# A count of a wrong page or a wrong report is no count.
for c in "$p8_counts" "$php_counts"; do
    case $c in *WRONG* | *FAILED*) fail "A count says WRONG or FAILED; nothing appended:
$c" ;; esac
done

# <counts> <app> <whole|execution>: the number on that line, without commas.
pick() {
    printf '%s\n' "$1" | awk -v app="$2" -v kind="$3" '$1 == app && $4 == kind { gsub(",", "", $3); print $3 }'
}
wp_x=$(pick "$p8_counts" wordpress execution)
wp_x_php=$(pick "$php_counts" wordpress execution)
lv_x=$(pick "$p8_counts" laravel execution)
lv_x_php=$(pick "$php_counts" laravel execution)
wp_w=$(pick "$p8_counts" wordpress whole)
lv_w=$(pick "$p8_counts" laravel whole)
for n in "$wp_x" "$wp_x_php" "$lv_x" "$lv_x_php" "$wp_w" "$lv_w"; do
    case $n in '' | *[!0-9]*) fail "A count is missing or not a number; nothing appended:
$p8_counts
$php_counts" ;; esac
done
ratio() { awk -v a="$1" -v b="$2" 'BEGIN { printf "%.2f", a / b }'; }

[ -e "$results" ] || printf 'date\tsha\twordpress execution p8\twordpress execution php\tratio\tlaravel execution p8\tlaravel execution php\tratio\twordpress whole run p8\tlaravel whole run p8\tcount label\tphp count label\n' > "$results"
row=$(printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s' "$day" "$sha" \
    "$wp_x" "$wp_x_php" "$(ratio "$wp_x" "$wp_x_php")" \
    "$lv_x" "$lv_x_php" "$(ratio "$lv_x" "$lv_x_php")" \
    "$wp_w" "$lv_w" "$label" "$plabel")
printf '%s\n' "$row" >> "$results" || fail "Could not write $results."
printf '%s\n' "$row"

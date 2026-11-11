#!/bin/sh
# Runs on simien (macOS) from ~/p8-lane-alloc. Wall ns + peak RSS
# (/usr/bin/time -l) for WordPress --repeat 1 (cold), --repeat 4 (warm =
# (r4 - r1)/3) and the Laravel test, p8-<candidate> each. Every round runs
# all candidates back to back, order rotated per round (the WordPress page's
# work drifts with the clock, #317). Appends to results.txt:
#   wall <workload> <candidate> <round> <wall_ns> <rss_kb> <md5 of stdout> | <check>
cd ~/p8-lane-alloc || exit 2
mkdir -p out
rounds=$1
CANDS=${CANDS:-"system dlmalloc rlsf talc rusty"}
now() { perl -MTime::HiRes=time -e 'printf "%d\n", time * 1e9'; }
fresh() { cp wp/pristine.sqlite wp/site/wp-content/database/.ht.sqlite; }
order() { # CANDS rotated left by round-1
  k=$1; set -- $CANDS; n=$(( (k - 1) % $# )); list=""
  i=0; for c in $CANDS; do [ $i -ge $n ] && list="$list $c"; i=$((i+1)); done
  i=0; for c in $CANDS; do [ $i -lt $n ] && list="$list $c"; i=$((i+1)); done
  echo $list
}
rsskb() { echo $(( $(grep -a "maximum resident set size" "$1" | awk '{print $1}') / 1024 )); }
run() { # run <workload> <candidate> <round>
  w=$1 c=$2 r=$3 o=$HOME/p8-lane-alloc/out/wall-$1-$2-$3
  case $w in
    wp-r1|wp-r4)
      fresh; n=${w#wp-r}
      t0=$(now)
      (cd wp && /usr/bin/time -l ../p8-$c --repeat $n render.php > $o.out 2> $o.err)
      t1=$(now)
      check=$(grep -a render_ms $o.err | sed 's/ bytes=.*//' | tr '\n' ' ')
      ;;
    lv)
      t0=$(now)
      (cd laravel && /usr/bin/time -l ../p8-$c vendor/bin/phpunit --do-not-cache-result tests/Support/SupportCollectionTest.php > $o.out 2> $o.err)
      t1=$(now)
      check=$(perl -pe 's/\e\[[0-9;]*m//g' $o.out | grep -aoE "Tests: [0-9]+.*|OK \([0-9]+ tests?, [0-9]+ assertions?\)" | tail -1 | cut -c1-90)
      ;;
  esac
  echo "wall $w $c $r $((t1 - t0)) $(rsskb $o.err) $(md5 -q $o.out | cut -c1-12) | $check" | tee -a results.txt
}
r=1
while [ $r -le $rounds ]; do
  for c in $(order $r); do
    run wp-r1 $c $r
    run wp-r4 $c $r
    run lv $c $r
  done
  r=$((r+1))
done
echo all-done

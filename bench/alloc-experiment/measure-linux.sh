#!/bin/sh
# Runs on addis from ~/p8-lane-alloc, inside `flock ~/p8-perf/.build.lock`
# and `nix shell nixpkgs#valgrind nixpkgs#time` (the lock is held for the whole
# script so wall times run on a quiet machine). Binaries: p8-<candidate>.
#   phase cg:   callgrind instructions; WordPress --repeat 1 (cold) and
#               --repeat 4 (warm = (r4 - r1)/3), Laravel suite (cold).
#   phase wall: wall ns + peak RSS outside valgrind, same three workloads.
# Each round runs every candidate back to back, the order rotated per round
# (the WordPress page's work drifts with the clock, #317).
# Appends one line per run to results.txt:
#   <phase> <workload> <candidate> <round> <instructions|wall_ns> <rss_kb> <md5 of stdout> | <check>
cd ~/p8-lane-alloc || exit 2
mkdir -p out
phase=$1 rounds=$2
CANDS=${CANDS:-"glibc dlmalloc rlsf talc rusty"}
LV=$HOME/p8-perf/laravel
ir() { grep "I   refs" "$1" | grep -o "[0-9,]*$" | tr -d ,; }
fresh() { cp wp/pristine.sqlite wp/site/wp-content/database/.ht.sqlite; }
order() { # order <round>: CANDS rotated left by round-1
  k=$1; set -- $CANDS; n=$(( (k - 1) % $# )); shift_list=""
  i=0; for c in $CANDS; do [ $i -ge $n ] && shift_list="$shift_list $c"; i=$((i+1)); done
  i=0; for c in $CANDS; do [ $i -lt $n ] && shift_list="$shift_list $c"; i=$((i+1)); done
  echo $shift_list
}
lvline() { grep -aoE "Tests: [0-9]+.*|OK \([0-9]+ tests?, [0-9]+ assertions?\)" "$1" | tail -1 | sed 's/\x1b\[[0-9;]*m//g' | head -c 90; }
run() { # run <workload> <candidate> <round>
  w=$1 c=$2 r=$3 o=out/$phase-$w-$c-$r
  case $w in
    wp-r1|wp-r4)
      fresh
      n=${w#wp-r}
      if [ $phase = cg ]; then
        (cd wp && valgrind --tool=callgrind --callgrind-out-file=../out/cg.tmp ../p8-$c --repeat $n render.php > ../$o.out 2> ../$o.err)
        val=$(ir $o.err) rss=-
      else
        t0=$(date +%s%N)
        (cd wp && time -f %M -o ../$o.rss ../p8-$c --repeat $n render.php > ../$o.out 2> ../$o.err)
        t1=$(date +%s%N); val=$((t1 - t0)) rss=$(tail -1 $o.rss)
      fi
      check=$(grep -a render_ms $o.err | sed 's/ bytes=.*//' | tr '\n' ' ')
      ;;
    lv)
      if [ $phase = cg ]; then
        (cd $LV && valgrind --tool=callgrind --callgrind-out-file=$HOME/p8-lane-alloc/out/cg.tmp $HOME/p8-lane-alloc/p8-$c vendor/bin/phpunit --do-not-cache-result tests/Support/SupportCollectionTest.php > $HOME/p8-lane-alloc/$o.out 2> $HOME/p8-lane-alloc/$o.err)
        val=$(ir $o.err) rss=-
      else
        t0=$(date +%s%N)
        (cd $LV && time -f %M -o $HOME/p8-lane-alloc/$o.rss $HOME/p8-lane-alloc/p8-$c vendor/bin/phpunit --do-not-cache-result tests/Support/SupportCollectionTest.php > $HOME/p8-lane-alloc/$o.out 2> $HOME/p8-lane-alloc/$o.err)
        t1=$(date +%s%N); val=$((t1 - t0)) rss=$(tail -1 $o.rss)
      fi
      check=$(lvline $o.out)
      ;;
  esac
  md5=$(md5sum < $o.out | cut -c1-12)
  echo "$phase $w $c $r $val $rss $md5 | $check" | tee -a results.txt
  rm -f out/cg.tmp
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

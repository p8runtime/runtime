#!/bin/sh
# The output-check variants of the micro programs: each one with an `echo`
# of the loop's result after the loop, so PHP 8.6 and the prototype can be
# compared on what they print (the measured programs print nothing).
# Writes micro/check/<name>.php from micro/<name>.php.
set -eu
cd "$(dirname "$0")/.."
mkdir -p micro/check
emit() {
  # $1 micro, $2 the expression echoed (the loop counter is echoed too)
  awk -v expr="$2" '
    /^}$/ && !done { print "    echo $i, \" \", " expr ", \"\\n\";"; done = 1 }
    { print }
  ' "micro/$1.php" > "micro/check/$1.php"
}
emit empty '"-"'
for m in add assign call call_typed method prop_read arr_read isset_dim builtin strlen concat identical; do emit $m '$x'; done
emit arr_write '$a[2]'
emit prop_write '$o->p'
emit prop_typed '$o->t'
emit new '$x->p'
ls micro/check

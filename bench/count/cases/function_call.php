<?php
// One call of a two-argument user function per iteration.
$n = (int) ($argv[1] ?? 1000);
function add($a, $b) { return $a + $b; }
function run($n) { $s = 0; for ($i = 0; $i < $n; $i++) { $s = add($s, $i); } return $s; }
echo run($n), "\n";

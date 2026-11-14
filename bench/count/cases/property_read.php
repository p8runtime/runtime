<?php
// One property read per iteration.
$n = (int) ($argv[1] ?? 1000);
class Q { public $v = 3; }
function run($n) { $q = new Q; $s = 0; for ($i = 0; $i < $n; $i++) { $s += $q->v; } return $s; }
echo run($n), "\n";

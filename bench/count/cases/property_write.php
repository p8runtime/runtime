<?php
// One property write per iteration.
$n = (int) ($argv[1] ?? 1000);
class Q { public $v = 0; }
function run($n) { $q = new Q; for ($i = 0; $i < $n; $i++) { $q->v = $i; } return $q->v; }
echo run($n), "\n";

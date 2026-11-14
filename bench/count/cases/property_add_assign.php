<?php
// One compound assignment to a property (`$p->x += $i`), no call.
$n = (int) ($argv[1] ?? 1000);
class P { public $x = 0; }
function run($n) { $p = new P; for ($i = 0; $i < $n; $i++) { $p->x += $i; } return $p->x; }
echo run($n), "\n";

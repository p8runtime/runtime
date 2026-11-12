<?php
// One variable-to-variable assignment per iteration.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $a = 1; $b = 0; for ($i = 0; $i < $n; $i++) { $b = $a; } return $b; }
echo run($n), "\n";

<?php
// One integer addition per iteration.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $s = 0; for ($i = 0; $i < $n; $i++) { $s = $s + $i; } return $s; }
echo run($n), "\n";

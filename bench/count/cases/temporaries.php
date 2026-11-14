<?php
// An expression with two temporaries (`($i + 1) * 2 - $i`).
$n = (int) ($argv[1] ?? 1000);
function run($n) { $s = 0; for ($i = 0; $i < $n; $i++) { $s = ($i + 1) * 2 - $i; } return $s; }
echo run($n), "\n";

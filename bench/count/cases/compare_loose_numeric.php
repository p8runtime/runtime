<?php
// One `==` of a numeric string against an int, in a condition.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $x = "10"; $s = 0; for ($i = 0; $i < $n; $i++) { if ($x == 10) { $s++; } } return $s; }
echo run($n), "\n";

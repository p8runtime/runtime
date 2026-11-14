<?php
// One `===` of a variable against a string constant, in a condition.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $status = "publish"; $s = 0; for ($i = 0; $i < $n; $i++) { if ($status === "publish") { $s++; } } return $s; }
echo run($n), "\n";

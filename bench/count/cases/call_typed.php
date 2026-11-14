<?php
// function_call with int parameter and return types.
$n = (int) ($argv[1] ?? 1000);
function f(int $a, int $b): int { return $a + $b; }
function run($n) { $s = 0; for ($i = 0; $i < $n; $i++) { $s = f($s, $i); } return $s; }
echo run($n), "\n";

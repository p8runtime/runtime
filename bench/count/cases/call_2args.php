<?php
// call_return plus two arguments (function_call without its addition).
$n = (int) ($argv[1] ?? 1000);
function f($a, $b) { return $a; }
function run($n) { $s = 0; for ($i = 0; $i < $n; $i++) { $s = f($s, $i); } return $s; }
echo run($n), "\n";

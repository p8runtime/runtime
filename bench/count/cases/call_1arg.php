<?php
// call_return plus one argument.
$n = (int) ($argv[1] ?? 1000);
function f($a) { return $a; }
function run($n) { $s = 0; for ($i = 0; $i < $n; $i++) { $s = f($i); } return $s; }
echo run($n), "\n";

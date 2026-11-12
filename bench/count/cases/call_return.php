<?php
// call_empty plus a returned value that is assigned.
$n = (int) ($argv[1] ?? 1000);
function f() { return 1; }
function run($n) { $s = 0; for ($i = 0; $i < $n; $i++) { $s = f(); } return $s; }
echo run($n), "\n";

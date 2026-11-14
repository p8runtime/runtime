<?php
// One call_user_func of a user function named by a string.
$n = (int) ($argv[1] ?? 1000);
function add($a, $b) { return $a + $b; }
function run($n) { $s = 0; for ($i = 0; $i < $n; $i++) { $s = call_user_func("add", $s, $i); } return $s; }
echo run($n), "\n";

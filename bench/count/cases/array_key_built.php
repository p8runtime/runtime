<?php
// One write under a key built from a string and a number.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $a = []; for ($i = 0; $i < $n; $i++) { $a["k" . ($i & 7)] = $i; } return count($a); }
echo run($n), "\n";

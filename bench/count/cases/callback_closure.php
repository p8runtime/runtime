<?php
// One array_map of a closure over a four-element list (four callbacks a builtin makes).
$n = (int) ($argv[1] ?? 1000);
function run($n) { $t = 0; $a = [1, 2, 3, 4]; for ($i = 0; $i < $n; $i++) { $t += count(array_map(fn($x) => $x + $i, $a)); } return $t; }
echo run($n), "\n";

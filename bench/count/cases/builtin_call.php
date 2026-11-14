<?php
// One builtin call (strlen) per iteration.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $t = 0; $s = "hello world"; for ($i = 0; $i < $n; $i++) { $t += strlen($s); } return $t; }
echo run($n), "\n";

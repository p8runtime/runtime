<?php
// The loop this harness was first measured on: `$s += $i & 7`.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $s = 0; for ($i = 0; $i < $n; $i++) { $s += $i & 7; } return $s; }
echo run($n), "\n";

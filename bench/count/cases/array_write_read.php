<?php
// One string-keyed array write and read per iteration.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $a = ["k" => 0]; $s = 0; for ($i = 0; $i < $n; $i++) { $a["k"] = $i; $s += $a["k"]; } return $s; }
echo run($n), "\n";

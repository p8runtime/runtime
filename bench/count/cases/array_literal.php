<?php
// One two-element array literal built per iteration.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $s = 0; for ($i = 0; $i < $n; $i++) { $a = ["a" => $i, "b" => 2]; $s += $a["b"]; } return $s; }
echo run($n), "\n";

<?php
// One read of a string-keyed element.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $a = ["key" => 3, "other" => 4]; $s = 0; for ($i = 0; $i < $n; $i++) { $s += $a["key"]; } return $s; }
echo run($n), "\n";

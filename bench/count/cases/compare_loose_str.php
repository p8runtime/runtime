<?php
// One `==` of two non-numeric strings, in a condition.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $status = "publish"; $s = 0; for ($i = 0; $i < $n; $i++) { if ($status == "draft") { $s++; } } return $s; }
echo run($n), "\n";

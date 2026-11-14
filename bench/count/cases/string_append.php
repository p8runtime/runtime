<?php
// One append to a growing string, reset every 64 iterations.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $t = 0; $s = ""; for ($i = 0; $i < $n; $i++) { $s .= "item"; if (($i & 63) === 63) { $t += strlen($s); $s = ""; } } return $t; }
echo run($n), "\n";

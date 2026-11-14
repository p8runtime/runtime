<?php
// One short string built per iteration.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $t = 0; for ($i = 0; $i < $n; $i++) { $s = "item " . $i . "!"; $t += strlen($s); } return $t; }
echo run($n), "\n";

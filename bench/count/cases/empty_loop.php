<?php
// An empty counted loop.
$n = (int) ($argv[1] ?? 1000);
function run($n) { for ($i = 0; $i < $n; $i++) { } return $i; }
echo run($n), "\n";

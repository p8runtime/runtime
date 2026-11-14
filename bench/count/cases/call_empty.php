<?php
// The bare cost of a call: an empty function, no arguments, result unused.
$n = (int) ($argv[1] ?? 1000);
function f() { }
function run($n) { for ($i = 0; $i < $n; $i++) { f(); } return $i; }
echo run($n), "\n";

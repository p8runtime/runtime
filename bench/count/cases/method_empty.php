<?php
// The bare cost of a method call: an empty method, no arguments.
$n = (int) ($argv[1] ?? 1000);
class P { function m() { } }
function run($n) { $p = new P; for ($i = 0; $i < $n; $i++) { $p->m(); } return $i; }
echo run($n), "\n";

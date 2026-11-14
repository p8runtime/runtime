<?php
// A method call with one argument and a returned value, no property access.
$n = (int) ($argv[1] ?? 1000);
class P { function m($a) { return $a; } }
function run($n) { $p = new P; $s = 0; for ($i = 0; $i < $n; $i++) { $s = $p->m($i); } return $s; }
echo run($n), "\n";

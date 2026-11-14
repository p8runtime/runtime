<?php
// A static method call with one argument and a returned value.
$n = (int) ($argv[1] ?? 1000);
class P { static function m($a) { return $a; } }
function run($n) { $s = 0; for ($i = 0; $i < $n; $i++) { $s = P::m($i); } return $s; }
echo run($n), "\n";

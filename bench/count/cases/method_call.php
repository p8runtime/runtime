<?php
// One method call per iteration.
$n = (int) ($argv[1] ?? 1000);
class P { private $x = 0; function inc($d) { $this->x += $d; return $this->x; } }
function run($n) { $p = new P; $s = 0; for ($i = 0; $i < $n; $i++) { $s = $p->inc($i); } return $s; }
echo run($n), "\n";

<?php
// One object made (constructor run) and dropped per iteration.
$n = (int) ($argv[1] ?? 1000);
class N { public $v; function __construct($v) { $this->v = $v; } }
function run($n) { $s = 0; for ($i = 0; $i < $n; $i++) { $o = new N($i); $s += $o->v; } return $s; }
echo run($n), "\n";

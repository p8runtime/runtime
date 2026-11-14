<?php
// One method call that reads a property on $this.
$n = (int) ($argv[1] ?? 1000);
class Q { public $v = 3; function get() { return $this->v; } }
function run($n) { $q = new Q; $s = 0; for ($i = 0; $i < $n; $i++) { $s += $q->get(); } return $s; }
echo run($n), "\n";

<?php
// One foreach over a 10-element keyed array (10 steps per iteration).
$n = (int) ($argv[1] ?? 1000);
function run($n) { $a = ["a"=>1,"b"=>2,"c"=>3,"d"=>4,"e"=>5,"f"=>6,"g"=>7,"h"=>8,"i"=>9,"j"=>10]; $s = 0; for ($i = 0; $i < $n; $i++) { foreach ($a as $k => $v) { $s += $v; } } return $s; }
echo run($n), "\n";

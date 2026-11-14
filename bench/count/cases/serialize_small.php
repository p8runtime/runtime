<?php
// One serialize() of a three-element array.
$n = (int) ($argv[1] ?? 1000);
function run($n) { $t = 0; $a = ["id" => 7, "name" => "post", "ok" => true]; for ($i = 0; $i < $n; $i++) { $t += strlen(serialize($a)); } return $t; }
echo run($n), "\n";

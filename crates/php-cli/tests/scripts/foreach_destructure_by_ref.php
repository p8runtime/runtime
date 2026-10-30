<?php
// foreach ($a as [&$x, 'k' => &$y]): destructuring by reference into each element.
$errors = [['type' => 'error', 'message' => 'a'], ['type' => 'warning', 'message' => 'b']];
foreach ($errors as ['type' => &$type, 'message' => $m]) { $type = strtoupper($type) . ":$m"; }
unset($type);
var_dump($errors);
$pairs = [[1, [2, 3]], [4, [5, 6]]];
foreach ($pairs as $k => [&$a, [, &$c]]) { $a *= 10; $c = "c$k"; }
unset($a, $c);
var_dump($pairs);
class H { public $rows = [['x' => 1], ['x' => 2]]; }
$h = new H;
foreach ($h->rows as ['x' => &$x]) { $x++; }
unset($x);
var_dump($h->rows);
$missing = [[], ['k' => 5]];
foreach ($missing as ['k' => &$k]) { $k = $k ?? 'new'; }
unset($k);
var_dump($missing);

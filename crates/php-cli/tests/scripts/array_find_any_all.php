<?php
// PHP 8.4's array_find, array_find_key, array_any, array_all: callback($value, $key), first match.
$a = ['x' => 1, 'y' => 4, 'z' => 9, 3 => 16];
$calls = [];
$even = function ($v, $k) use (&$calls) { $calls[] = "$k=$v"; return $v % 2 === 0; };
var_dump(array_find($a, $even), array_find_key($a, $even), array_any($a, $even), array_all($a, $even));
echo implode(',', $calls), "\n";
var_dump(array_find([], fn() => true), array_find_key($a, fn($v) => $v > 100), array_any([], fn() => true), array_all([], fn() => false));
var_dump(array_all([2, 4], fn($v) => $v % 2 === 0), array_find_key([5 => 'a', 'k' => 'b'], fn($v, $k) => $k === 'k'));

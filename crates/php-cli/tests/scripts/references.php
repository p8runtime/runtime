<?php
$a = 1; $b = &$a; $b = 2; echo $a, "\n";
unset($b); $b = 3; echo $a, "\n";
$arr = ['x' => 1, 'y' => [1, 2]];
$r = &$arr['x']; $r = 10;
$s = &$arr['y'][1]; $s *= 5;
$n = &$arr['new']['deep']; $n = 'made';
print_r($arr);
$v = 'val'; $list = [];
$list[] = &$v; $list['k'] = &$v; $v = 'changed';
print_r($list);
$data = [1, 2, 3];
foreach ($data as &$item) { $item *= 2; }
unset($item);
print_r($data);
$m = ['a' => 1, 'b' => 2];
foreach ($m as $k => &$x) { $x = "$k$x"; }
print_r($m);
function counter() { static $c = 0; return ++$c; }
counter(); counter(); echo counter(), "\n";
function init() { static $calls = [], $n; $calls[] = count($calls); $n++; return [count($calls), $n]; }
init(); print_r(init());
class K { function m() { static $x = 10; return $x++; } }
$k1 = new K; $k2 = new K; echo $k1->m(), $k2->m(), $k1->m(), "\n";
$stack = [3, 1, 2]; $wrap = ['s' => [5, 4]];
sort($wrap['s']); print_r($wrap);
array_push($wrap['s'], 6); echo count($wrap['s']), "\n";
function addOne(array &$xs) { $xs[] = 1; }
$h = ['l' => []]; addOne($h['l']); addOne($h['l']); echo count($h['l']), "\n";
$copy = $data; $ref = &$data[0]; $ref = 'r'; echo $copy[0], $data[0], "\n";
$x1 = 1; $x2 = &$x1; $x2++; echo $x1, "\n";
$g = 5; function useGlobal() { global $g; $g++; } useGlobal(); echo $g, "\n";
$fns = []; for ($i = 0; $i < 3; $i++) { $fns[] = function() use (&$i) { return $i; }; } echo $fns[0](), "\n";

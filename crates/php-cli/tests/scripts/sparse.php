<?php
// Int-keyed arrays that are not 0..n-1: holes, unsets, re-adds, appends.
$a = [];
for ($i = 1; $i <= 5; $i++) { $a[$i] = $i * 10; }
var_dump($a, count($a), isset($a[0]), isset($a[3]));
unset($a[3]);
var_dump($a, count($a));
$a[3] = 33;
foreach ($a as $k => $v) { echo "$k=$v "; }
echo "\n";
$b = [10, 20, 30];
unset($b[2]);
$b[] = 40;
unset($b[0]);
$b[] = 50;
var_dump($b);
$c = [];
$c[2] = "two";
$c[0] = "zero";
$c[] = "next";
var_dump($c);
$d = [-3 => "neg"];
$d[] = "after";
var_dump($d);
$e = [5 => "a"];
$e[1000] = "far";
$e[] = "more";
var_dump($e);
$f = [0, 1, 2, 3];
foreach ($f as $k => $v) { if ($k == 1) { unset($f[2]); } echo $v; }
echo "\n";
var_dump($f);
$g = [];
$g[7] = 1;
echo isset($g[6]) ? "y" : "n", isset($g[7]) ? "y" : "n", "\n";
echo $g[6] ?? "default", "\n";
var_dump($g[6]);

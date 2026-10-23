<?php
$a = [1, 2, 3];
$a[1] = 20; $a[3] = 4; $a[10] = 5; $a[-1] = 6;
var_dump($a[0], $a[1], $a[3], $a[10], $a[-1]);
var_dump($a[99]);
$b = $a; $b[0] = 100; var_dump($a[0], $b[0]);
function fill(&$arr, $n) { for ($i = 0; $i < $n; $i++) $arr[$i] = $i * 1.5; }
$c = []; fill($c, 4); var_dump($c);
function first(&$arr) { return $arr[0] + $arr[1]; }
var_dump(first($c));
$s = "abc"; var_dump($s[1], $s[-1]);
$f = ["x" => 1]; var_dump($f[0] ?? "none");
$g = null; $g[2] = 3; var_dump($g);
$h = [5 => 1]; $h[] = 2; var_dump($h);
$m = [[1, 2], [3, 4]]; $m[1][0] = 30; var_dump($m[1][0], $m[0][1]);
$i = 0; $p = []; $p[$i] = $i++; var_dump($p);
$q = [1.5, true, null]; var_dump($q[0] + $q[1], $q[2]);
$u = [1, 2]; $u[1] = "str"; $u[0] = [9]; var_dump($u);
$w = [0 => 1]; $w[0] += 5; var_dump($w);
$ints = []; for ($k = 0; $k < 5; $k++) { $ints[$k] = $k * $k; } var_dump($ints[4]);
$copy = $ints; for ($k = 0; $k < 5; $k++) { $copy[$k] = -$k; } var_dump($ints[3], $copy[3]);
var_dump($ints[1.7]);

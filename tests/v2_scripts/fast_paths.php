<?php
// int overflow leaves the fast path for float
$i = PHP_INT_MAX; $i++; var_dump($i);
$j = PHP_INT_MIN; $j--; var_dump($j);
$a = PHP_INT_MAX; $b = $a + 1; var_dump($b);
$c = PHP_INT_MIN; $d = $c - 1; var_dump($d);
$e = PHP_INT_MAX; $f = $e * 2; var_dump($f);
// mixed int/float arithmetic and comparisons
$x = 3; $y = 1.5; var_dump($x + $y, $x - $y, $x * $y, $x / $y, $y / 0.5);
var_dump($x < $y, $x > $y, $x == 3.0, $x === 3.0, $x !== 3.0, 3.0 === 3.0);
$n = NAN; var_dump($n == $n, $n < 1, $n > 1, $n != $n, 1 <=> $n);
for ($k = 0.5; $k < 3; $k = $k + 1) { echo $k, "\n"; }
if ($n < 1) { echo "nan<1\n"; } else { echo "not nan<1\n"; }
while ($n == $n) { echo "never\n"; }
// assignment through references
function set5(&$s) { $s = 5; }
$r = 1; set5($r); var_dump($r);
function setf(&$s) { $s = 2.5; $s = $s * 2; }
$t = 1; setf($t); var_dump($t);
function bump(&$s) { $s++; $s += 1; }
$v = 1; bump($v); var_dump($v);
function gl() { global $g; $g = 7; $g++; }
gl(); var_dump($g);
// undefined variables
var_dump($undef1 + 1);
var_dump($undef2 < 1);
if ($undef3 == null) { echo "null\n"; }

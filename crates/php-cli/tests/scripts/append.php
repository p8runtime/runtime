<?php
// .= and ++/-- whose value is used.
$s = "";
for ($i = 0; $i < 3; $i++) { $s .= "ab"; }
var_dump($s);
$a = "x";
$b = $a;
$b .= "y";
var_dump($a, $b);
$c = "hi";
$c .= $c;
var_dump($c);
$d = "n=";
$d .= 5;
$d .= 1.5;
$d .= true;
$d .= null;
var_dump($d);
$e = 10;
$e .= "!";
var_dump($e);
$f .= "new";
var_dump($f);
$arr = ["k" => "v"];
$g = $arr["k"];
$g .= "w";
var_dump($arr["k"], $g);
$n = 3;
while ($n-- > 0) { echo $n; }
echo "\n";
var_dump($n);
$m = 0;
echo $m++, $m++, ++$m, --$m, $m--, "\n";
var_dump($m);
$big = PHP_INT_MAX;
$x = $big++;
var_dump($x, $big);
$small = PHP_INT_MIN;
$y = --$small;
var_dump($y);
$z = null;
$w = $z++;
var_dump($w, $z);
$str = "a9";
$v = $str++;
var_dump($v, $str);

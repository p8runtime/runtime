<?php
// $x = a op b and $x op= b, into plain variables and into references.
$g = 1;
function bump_global() {
    global $g;
    $g = $g + 10;
    $g = $g * 2;
    $g -= 1;
    $g = $g > 20;
}
bump_global();
var_dump($g);
// Only the destination is a reference: the result goes through it.
function set_global() {
    global $g;
    $t = 4;
    $g = $t * 3;
    var_dump($g);
    $g = $t < 5;
}
set_global();
var_dump($g);
function add_into(&$x, $y) {
    $x = $x + $y;
    $x = $x . "!";
}
$v = 5;
add_into($v, 3);
var_dump($v);
function set_ref(&$x, $y) {
    $x = $y * 2;
}
$w = 1;
set_ref($w, 21);
var_dump($w);
$arr = [3, 1, 2];
$s = 0;
foreach ($arr as $e) {
    $s = $s + $e * 10;
}
var_dump($s);
$i = PHP_INT_MAX;
$i = $i + 1;
var_dump($i);
$j = 7;
$j = $j % 3;
$k = 2 ** 10;
$k = $k <=> 1000;
$str = "a";
$str = $str . "b" . $str;
$cmp = $j < $k;
var_dump($j, $k, $str, $cmp);
$u = $undefined + 1;
var_dump($u);
$d = 10;
$d /= 4;
$d .= "x";
var_dump($d);

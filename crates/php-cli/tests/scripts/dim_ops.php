<?php
// $a[$k] op= $v: the common number cases and everything around them.
$a = [1, 2, 3];
for ($i = 0; $i < 3; $i++) {
    $a[$i] += 10;
    $a[$i] *= 2;
    $a[$i] -= 1;
}
var_dump($a);
$f = [1.5, 2];
$f[0] += 1;
$f[1] += 0.25;
$f[1] /= 2;
var_dump($f);
$big = [PHP_INT_MAX];
$big[0] += 1;
var_dump($big);
$copy = $a;
$copy[0] += 100;
var_dump($a[0], $copy[0]);
$s = ["5", "x"];
$s[0] += 1;
var_dump($s);
$m = [];
$m[3] += 1;
var_dump($m);
$sum = [0];
foreach ([1, 2, 3] as $v) {
    $sum[0] += $v;
}
echo $sum[0], "\n";
// String keys, numeric-string keys (the same as int keys) and misses.
$h = ["a" => 1, "b" => 2.5, "7" => 3];
$h["a"] += 10;
$h["b"] *= 2;
$h["7"] -= 1;
$h[7] += 100;
$h["c"] += 1;
var_dump($h);
$copyh = $h;
$copyh["a"] += 1;
var_dump($h["a"], $copyh["a"]);

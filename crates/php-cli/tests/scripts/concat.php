<?php
// . and interpolation: every kind of value, chains, order of effects.
function t($v) { echo "[t]"; return $v; }
$i = 42; $n = -7; $f = 1.5; $big = 1e20; $z = 0.1 + 0.2;
echo "i=$i n=$n f=$f big=$big z=$z\n";
echo "a" . 1 . 2.0 . true . false . null . "b" . PHP_INT_MIN . "\n";
$s = "x";
$s = $s . "y";
$s = $s . $s;
echo $s, "\n";
$copy = $s;
$s = $s . "!";
echo $copy, " ", $s, "\n";
echo t("a") . t("b") . t("c"), "\n";
$arr = [1];
echo "arr: " . $arr . " end\n";
echo "interp $arr[0] and {$arr[0]}\n";
$u = "pre-" . $undefined . "-post";
echo $u, "\n";
$out = "";
for ($k = 0; $k < 3; $k++) {
    $name = "user$k";
    $out .= "<li>$name: " . ($k * 2) . "</li>";
}
echo $out, "\n";
$num = 5;
$num = $num . "";
var_dump($num);

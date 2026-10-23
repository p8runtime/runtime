<?php
// foreach by value: keys, values, nesting, changes during the loop.
$a = ["x" => 1, "y" => 2, 5 => 3, 9 => "s"];
foreach ($a as $k => $v) {
    echo $k, "=", $v, "\n";
}
foreach ([10, 20, 30] as $v) {
    echo $v, " ";
}
echo "\n";
foreach ([] as $v) {
    echo "never\n";
}
$sum = [];
$src = ["a" => 1, "b" => 2];
foreach ([1, 2, 3] as $round) {
    foreach ($src as $k => $v) {
        $sum[$k] = ($sum[$k] ?? 0) + $v * $round;
    }
}
var_dump($sum);
// The loop walks the array as it was when the loop began.
$grow = [1, 2];
foreach ($grow as $v) {
    $grow[] = $v * 10;
}
var_dump($grow);
$k = "unchanged";
$v = "unchanged";
foreach ([7 => "seven"] as $k => $v) {
}
var_dump($k, $v);
foreach (5 as $v) {
}
echo "after\n";

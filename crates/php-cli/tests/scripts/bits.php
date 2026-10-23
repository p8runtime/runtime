<?php
$vals = [0, 1, -1, 7, -7, 64, PHP_INT_MAX, PHP_INT_MIN];
$shifts = [0, 1, 3, 62, 63, 64, 65, 100];
foreach ($vals as $x) {
    foreach ($shifts as $s) { echo $x << $s, " ", $x >> $s, " "; }
    foreach ($vals as $y) { echo $x & $y, " ", $x | $y, " ", $x ^ $y, " "; if ($y != 0) { echo $x % $y, " "; } }
    echo "\n";
}

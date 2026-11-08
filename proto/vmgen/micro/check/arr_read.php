<?php
function f($a) { return $a; }
function g(int $a): int { return $a; }
class P { public $p = 0; public int $t = 0; function m($a) { return $a; } }
function t($n) {
    $a = [1, 2, 3, 4];
    for ($i = 0; $i < $n; $i++) {
        $x = $a[2];
    }
    echo $i, " ", $x, "\n";
}
t((int) $argv[1]);

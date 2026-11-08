<?php
function f($a) { return $a; }
function g(int $a): int { return $a; }
class P { public $p = 0; public int $t = 0; function m($a) { return $a; } }
function t($n) {
    $s = 'hello';
    for ($i = 0; $i < $n; $i++) {
        $x = strlen($s);
    }
}
t((int) $argv[1]);

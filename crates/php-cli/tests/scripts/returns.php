<?php
// Returns reached through jumps: ternaries, if/else, loops, nesting.
function fibo_r($n) { return ($n < 2) ? 1 : fibo_r($n - 2) + fibo_r($n - 1); }
function sign($x) { return $x > 0 ? 1 : ($x < 0 ? -1 : 0); }
function pick($c, $a, $b) { return $c ? $a : $b; }
function str($c) { return $c ? "yes" : "no"; }
function firstOver($xs, $limit) {
    foreach ($xs as $x) {
        if ($x > $limit) {
            return $x;
        }
    }
    return null;
}
function either($c) {
    if ($c) {
        return 1.5;
    } else {
        return "two";
    }
}
function nothing($c) {
    if ($c) {
        return;
    }
    echo "fell through\n";
}
function undef($c) { return $c ? $missing : 3; }
echo fibo_r(10), "\n";
echo sign(5), sign(-5), sign(0), "\n";
echo pick(true, "a", "b"), pick(false, "a", "b"), "\n";
echo str(1), str(0), "\n";
var_dump(firstOver([1, 5, 9], 4), firstOver([1, 2], 4));
var_dump(either(true), either(false));
var_dump(nothing(true));
var_dump(nothing(false));
var_dump(undef(false));
var_dump(undef(true));

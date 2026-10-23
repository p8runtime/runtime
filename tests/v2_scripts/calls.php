<?php
function add($a, $b = 10) { return $a + $b; }
var_dump(add(1), add(1, 2));
function inc(&$x) { $x++; }
$v = 5; inc($v); var_dump($v);
function args() { return func_get_args(); }
var_dump(args(1, "two", 3.0));
function rec($n) { return $n <= 1 ? 1 : $n * rec($n - 1); }
var_dump(rec(10), rec(25));
function g() { global $gv; $gv++; return $gv; }
$gv = 41; var_dump(g(), $gv);
function noret() {}
var_dump(noret());
function fib($n) { return $n < 2 ? $n : fib($n - 1) + fib($n - 2); }
var_dump(fib(20));
function locals() { $big = range(1, 3); return count($big); }
var_dump(locals(), locals());
echo strlen("hello"), "\n";
function later() { return early() . "!"; }
function early() { return "early"; }
echo later(), "\n";
function deep($n) { return $n == 0 ? 0 : 1 + deep($n - 1); }
var_dump(deep(5000));

<?php
$a = 1; $b = [2]; $c = 'x';
$f = function ($p) use ($a, &$b, $c) { static $s = 5; return $p; };
$b[] = 3;
var_dump((new ReflectionFunction($f))->getClosureUsedVariables());
var_dump((new ReflectionFunction(fn () => $a + 1))->getClosureUsedVariables());
var_dump((new ReflectionFunction(function () { static $z = 1; }))->getClosureUsedVariables());
var_dump((new ReflectionFunction('strlen'))->getClosureUsedVariables());
function named() {}
var_dump((new ReflectionFunction('named'))->getClosureUsedVariables());
class K { function m() { $q = 4; return function () use ($q) {}; } }
var_dump((new ReflectionFunction((new K)->m()))->getClosureUsedVariables());

<?php
// ReflectionFunctionAbstract::getStaticVariables: constant initializers
// before the first call, values after; a closure's use values first.
function sc($p) { static $cache = []; static $n = 1; $cache[$p] = strtoupper($p); $n++; return 1; }
var_dump((new ReflectionFunction('sc'))->getStaticVariables());
sc('a'); sc('b');
var_dump((new ReflectionFunction('sc'))->getStaticVariables());
class K { function m() { static $calls = 0; $calls++; } }
(new K)->m(); (new K)->m();
var_dump((new ReflectionMethod('K', 'm'))->getStaticVariables());
$y = 5; $z = 'zz';
$f = function () use ($y, &$z) { static $hits = 0; $hits++; };
$f(); $f();
var_dump((new ReflectionFunction($f))->getStaticVariables());

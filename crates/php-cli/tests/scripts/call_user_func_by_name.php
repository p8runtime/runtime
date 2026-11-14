<?php
// call_user_func / call_user_func_array on a function's name, from a call
// site: what the builtin's path does (arguments by value, a list's
// references kept, the callback's errors), with the name looked up once.
function add($a, $b) { return $a + $b; }
function inc(&$x) { $x++; return $x; }
function args() { return func_get_args(); }
function counter() { static $n = 0; return ++$n; }
function gen($n) { for ($i = 0; $i < $n; $i++) { yield $i; } }
function boom($m) { throw new RuntimeException($m); }
function fact($n) { return $n <= 1 ? 1 : $n * call_user_func('fact', $n - 1); }
function nested($n) { return call_user_func_array('add', [call_user_func('add', $n, 1), 10]); }

$s = 0;
for ($i = 0; $i < 5; $i++) { $s = call_user_func('add', $s, $i); }
echo $s, "\n";
echo call_user_func('ADD', 2, 3), ' ', call_user_func('\add', 4, 5), ' ', call_user_func_array('Add', [6, 7]), "\n";

// By value where the parameter takes a reference: PHP's warning, the
// variable unchanged; the list's reference is passed as one.
$v = 1;
var_dump(call_user_func('inc', $v), $v);
$r = 1;
$list = [&$r];
var_dump(call_user_func_array('inc', $list), $r);
$w = 5;
$ref = &$w;
var_dump(call_user_func('inc', $ref), $w);

var_dump(call_user_func('args', 1, 'two', [3]));
var_dump(call_user_func_array('args', [5 => 'x', 9 => 'y']));
echo call_user_func('counter'), call_user_func('counter'), call_user_func_array('counter', []), "\n";
foreach (call_user_func('gen', 3) as $g) { echo $g; }
echo "\n";
echo call_user_func('fact', 6), ' ', nested(1), "\n";

try {
    call_user_func('boom', 'from the callback');
} catch (RuntimeException $e) {
    echo get_class($e), ': ', $e->getMessage(), ' at line ', $e->getLine(), "\n";
}
try {
    call_user_func('later', 1);
} catch (TypeError $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
if (true) {
    function later($x) { return "later $x"; }
}
echo call_user_func('later', 2), "\n";
try {
    call_user_func_array('add', 'not a list');
} catch (TypeError $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
try {
    call_user_func('add', 1);
} catch (ArgumentCountError $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
echo call_user_func('strtoupper', 'builtin'), ' ', call_user_func_array('str_repeat', ['ab', 2]), "\n";
$name = 'add';
echo call_user_func($name, 20, 22), ' ', call_user_func_array($name, [1, 1]), "\n";

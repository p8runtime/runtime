<?php
// Calls with more than 255 arguments pass them all, on every path:
// spread calls, call_user_func(_array) of methods and closures, a
// builtin given them, named ones after them (#580).

function f(...$a) { return count($a) . ":" . array_sum($a); }
function g() { return func_num_args() . ":" . array_sum(func_get_args()); }
function fixed($a, $b, ...$rest) { return "$a,$b+" . count($rest); }
function &refs(&...$a) { foreach ($a as &$v) { $v++; } $r = count($a); return $r; }

class O {
    function m(...$a) { return count($a) . ":" . array_sum($a); }
    static function s(...$a) { return count($a) . ":" . array_sum($a); }
    function __call($n, $a) { return "$n " . count($a); }
    static function __callStatic($n, $a) { return "static $n " . count($a); }
}
class K {
    public $n;
    function __construct(...$a) { $this->n = count($a); }
}

$l = range(1, 300);
$o = new O();
$cl = fn(...$a) => count($a) . ":" . array_sum($a);
$fn = 'f';

echo "cufa f: ", call_user_func_array('f', $l), "\n";
echo "cufa g: ", call_user_func_array('g', $l), "\n";
echo "cufa [o,m]: ", call_user_func_array([$o, 'm'], $l), "\n";
echo "cufa O::s: ", call_user_func_array('O::s', $l), "\n";
echo "cufa [O,s]: ", call_user_func_array(['O', 's'], $l), "\n";
echo "cufa closure: ", call_user_func_array($cl, $l), "\n";
echo "cufa __call: ", call_user_func_array([$o, 'zz'], $l), "\n";
echo "cufa __callStatic: ", call_user_func_array(['O', 'zz'], $l), "\n";
echo "cufa max: ", call_user_func_array('max', $l), "\n";
echo "cufa array_merge: ", count(call_user_func_array('array_merge', array_fill(0, 300, [1]))), "\n";
echo "cufa named: ", call_user_func_array('f', $l + ['x' => 1000]), "\n";
echo "cufa fixed: ", call_user_func_array('fixed', $l), "\n";
echo "cuf f: ", call_user_func('f', ...$l), "\n";
echo "cuf [o,m]: ", call_user_func([$o, 'm'], ...$l), "\n";
echo "cuf closure: ", call_user_func($cl, ...$l), "\n";
echo "spread f: ", f(...$l), "\n";
echo "spread g: ", g(...$l), "\n";
echo "spread m: ", $o->m(...$l), "\n";
echo "spread s: ", O::s(...$l), "\n";
echo "spread __call: ", $o->zz(...$l), "\n";
echo "spread __callStatic: ", O::zz(...$l), "\n";
echo "spread closure: ", $cl(...$l), "\n";
echo "spread dynamic: ", $fn(...$l), "\n";
echo "spread fixed: ", fixed(...$l), "\n";
echo "spread named: ", f(...$l, ...['x' => 1000]), "\n";
echo "spread new: ", (new K(...$l))->n, "\n";
echo "spread max: ", max(...$l), "\n";
echo "spread array_merge: ", count(array_merge(...array_fill(0, 300, [1]))), "\n";
echo "spread sprintf: ", strlen(sprintf(str_repeat("%d", 300), ...$l)), "\n";
echo "spread array_push: ", (function () use ($l) { $a = []; return array_push($a, ...$l); })(), "\n";
$r = array_fill(0, 300, 1);
$refs = [];
foreach ($r as $k => &$v) { $refs[$k] = &$v; }
unset($v);
echo "spread by ref: ", refs(...$refs), " ", array_sum($r), "\n";
echo "reflection invokeArgs: ", (new ReflectionFunction('f'))->invokeArgs($l), "\n";
echo "reflection invoke: ", (new ReflectionFunction('f'))->invoke(...$l), "\n";
echo "reflection method: ", (new ReflectionMethod('O', 'm'))->invokeArgs($o, $l), "\n";
echo "Closure::call: ", Closure::fromCallable([$o, 'm'])->call($o, ...$l), "\n";
echo "__invoke: ", $cl->__invoke(...$l), "\n";
echo "literal: ", eval('return f(' . implode(', ', $l) . ');'), "\n";
echo "literal method: ", eval('return $o->m(' . implode(', ', $l) . ');'), "\n";
echo "literal builtin: ", eval('return max(' . implode(', ', $l) . ');'), "\n";
$fiber = new Fiber(function (...$a) { return count($a); });
$fiber->start(...$l);
echo "fiber: ", $fiber->getReturn(), "\n";
echo "array_map: ", implode(",", array_map(fn(...$a) => count($a), ...array_fill(0, 300, [1]))), "\n";

<?php
// A closure made inside a closure is named after that closure alone (no
// class, no "()"); a closure's ArgumentCountError names the class it runs
// bound to (Closure::call's object's, bind's scope).

function short($name) { return str_replace(__FILE__, 'FILE', $name); }
function too_few(callable $f) {
    try { $f(); } catch (ArgumentCountError $e) { echo short(explode(',', $e->getMessage())[0]), "\n"; }
}

$f = function () { return function ($a) {}; };
echo short((new ReflectionFunction($f()))->getName()), "\n";
$g = fn() => fn($a) => 1;
echo short((new ReflectionFunction($g()))->getName()), "\n";
$h = function () { return fn() => function ($a) {}; };
echo short((new ReflectionFunction($h()()))->getName()), "\n";
function outer() { return function () { return function ($a) {}; }; }
echo short((new ReflectionFunction(outer()))->getName()), "\n";
echo short((new ReflectionFunction(outer()()))->getName()), "\n";
class C {
    function m() { return function () { return function ($a) {}; }; }
    static function s() { return function ($a) {}; }
}
echo short((new ReflectionFunction((new C)->m()))->getName()), "\n";
echo short((new ReflectionFunction((new C)->m()()))->getName()), "\n";
too_few((new C)->m()());
too_few(C::s());
too_few($f());

class M { public $p = 1; }
$k = function ($a) { return $this->p; };
too_few(fn() => $k->call(new M));
too_few(fn() => C::s()->call(new M));
too_few(Closure::bind($k, new M, M::class));
too_few(Closure::bind(C::s(), null, M::class));
too_few(C::s()->bindTo(null, null));
echo short((new ReflectionFunction(Closure::bind($k, new M, M::class)))->getName()), "\n";
try { (new C)->m()()(); } catch (ArgumentCountError $e) { echo short($e->getMessage()), "\n"; }

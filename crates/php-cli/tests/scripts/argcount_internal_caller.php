<?php
// Too few arguments: the message names the caller's file and line when
// PHP code made the call, and none when a builtin did (a callback,
// Reflection, Closure::__invoke) (#581).

function two($a, $b) { return $a; }
class M {
    function two($a, $b) { return $a; }
    static function stwo($a, $b) { return $a; }
}
class N {
    function __construct($a, $b) {}
}
$c2 = function ($a, $b) {};
$c3 = fn($x, $y, $z) => 0;

function attempt(string $name, callable $t): void {
    try {
        $t();
        echo "$name: no error\n";
    } catch (ArgumentCountError $e) {
        echo "$name: ", $e->getMessage(), "\n";
    }
}

// Called by a builtin.
attempt('array_map name', fn() => array_map('two', [1]));
attempt('array_map closure', fn() => array_map($c2, [1]));
attempt('array_map method', fn() => array_map([new M(), 'two'], [1]));
attempt('array_map static', fn() => array_map('M::stwo', [1]));
attempt('array_filter', fn() => array_filter([1], 'two'));
attempt('usort', function () use ($c3) { $a = [2, 1]; usort($a, $c3); });
attempt('array_walk', function () use ($c3) { $a = [1]; array_walk($a, $c3); });
attempt('array_reduce', fn() => array_reduce([1], $c3));
attempt('preg_replace_callback', fn() => preg_replace_callback('/a/', 'two', 'a'));
attempt('iterator_apply', fn() => iterator_apply(new ArrayIterator([1]), 'two'));
attempt('ReflectionFunction::invoke', fn() => (new ReflectionFunction('two'))->invoke(1));
attempt('ReflectionFunction::invokeArgs', fn() => (new ReflectionFunction('two'))->invokeArgs([1]));
attempt('ReflectionMethod::invoke', fn() => (new ReflectionMethod('M', 'two'))->invoke(new M(), 1));
attempt('Closure::__invoke', fn() => $c2->__invoke(1));

// Called by PHP code (call_user_func included: it is compiled as a call).
attempt('direct', fn() => two(1));
attempt('spread', fn() => two(...[1]));
attempt('dynamic', function () { $f = 'two'; return $f(1); });
attempt('method', fn() => (new M())->two(1));
attempt('static', fn() => M::stwo(1));
attempt('closure', fn() => $c2(1));
attempt('call_user_func', fn() => call_user_func('two', 1));
attempt('call_user_func_array', fn() => call_user_func_array('two', [1]));
attempt('call_user_func method', fn() => call_user_func([new M(), 'two'], 1));
attempt('call_user_func closure', fn() => call_user_func_array($c2, [1]));
attempt('new', fn() => new N(1));

// Called by PHP code inside a callback: that code's line.
attempt('inside a callback', fn() => array_map(fn($x) => two($x), [1]));

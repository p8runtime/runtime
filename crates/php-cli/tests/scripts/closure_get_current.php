<?php
// Closure::getCurrent() (PHP 8.5): the Closure object the calling code
// runs as, however it was called; in a generator's code, the closure the
// generator was made by.

$c = function () { return Closure::getCurrent(); };
var_dump($c() === $c);
var_dump(call_user_func($c) === $c);
var_dump(call_user_func_array($c, []) === $c);
var_dump($c->__invoke() === $c);
var_dump(array_map(fn ($x) => Closure::getCurrent(), [1])[0] instanceof Closure);

// Two Closure objects of one closure expression: each is its own.
$make = fn () => function () { return Closure::getCurrent(); };
[$a, $b] = [$make(), $make()];
var_dump($a() === $a, $b() === $b, $a() !== $b);

// A bound copy answers for itself, not for the closure it was bound from.
class Box { public $v = 7; }
$bound = Closure::bind(function () { return [Closure::getCurrent(), $this->v]; }, new Box, Box::class);
[$self, $v] = $bound();
var_dump($self === $bound, $v);

// Inside a generator made by a closure, at every resume.
$g = function () {
    yield Closure::getCurrent();
    yield Closure::getCurrent();
};
$gen = $g();
var_dump($gen->current() === $g);
$gen->next();
var_dump($gen->current() === $g);

// The generator keeps its closure alive.
$gen = (function () { yield 1; yield Closure::getCurrent(); })();
$gen->next();
var_dump($gen->current() instanceof Closure);

// A closure's code called from a plain function in it is not the closure.
function plain() { return Closure::getCurrent(); }
$outer = function () { return plain(); };
try {
    $outer();
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}

// A generator function that is not a closure.
function gen() { yield Closure::getCurrent(); }
try {
    gen()->current();
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}

// After a closure returns, the code that called it is not the closure.
$c();
try {
    Closure::getCurrent();
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}

// A method called from a closure is not the closure.
class M { static function m() { return Closure::getCurrent(); } }
try {
    (function () { return M::m(); })();
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}

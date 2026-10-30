<?php
class A {
    static function make() { return function () {}; }
    static function makeStatic() { return static fn() => static::class; }
    function bound() { return function () {}; }
}
class B extends A {}
$show = fn($c) => ($r = (new ReflectionFunction($c))->getClosureCalledClass()) ? $r->getName() : null;
var_dump($show(B::make()), $show(B::makeStatic()), $show((new B)->bound()), $show(function () {}));
var_dump($show(Closure::bind(function () {}, null, A::class)));
var_dump($show(Closure::fromCallable([B::class, 'make'])));
var_dump($show(strlen(...)));

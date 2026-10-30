<?php
// newInstanceWithoutConstructor ignores the constructor's visibility (Doctrine
// Instantiator makes PHPUnit mocks of classes with private constructors this way).
class P { private function __construct() { echo "ctor ran\n"; } public $x = 1; }
class Q extends P { }
final class Inst {
    function make($c) { $f = [new ReflectionClass($c), 'newInstanceWithoutConstructor']; return $f(); }
    function viaUnserialize($c) { return unserialize(sprintf('O:%d:"%s":0:{}', strlen($c), $c)); }
}
$i = new Inst;
var_dump($i->make('P'), $i->make('Q'), $i->viaUnserialize('Q'));
$o = (new ReflectionClass('Q'))->newInstanceWithoutConstructor(); var_dump($o->x);

<?php
// `new`, its constructor and static calls, each run twice or more so the
// second run takes the site's cache (the loop's arms).
class D { public $n; function __construct($n) { $this->n = $n; } function __destruct() { echo "~{$this->n}\n"; } }
class NoCtor { public $p = [1, 2]; }
class WithCtor {
    public $a; public $b;
    function __construct($a, $b = 'dflt') { $this->a = $a; $this->b = $b; echo "ctor $a $b\n"; }
}
class PrivCtor {
    private function __construct() { echo "priv\n"; }
    static function make() { return new PrivCtor; }
}
class Base {
    static function who() { return static::class; }
    static function make() { return new static; }
    function inst() { return "inst " . static::class . " " . get_class($this); }
    static function gen() { yield 1; yield 2; }
    static function __callStatic($m, $args) { return "callStatic $m " . count($args); }
}
class Child extends Base {
    static function viaParent() { return parent::who(); }
    static function viaSelf() { return self::who(); }
    function callParentInst() { return parent::inst(); }
}
abstract class Abs { }
interface Ifc { }

for ($i = 0; $i < 3; $i++) {
    // No constructor: arguments are still evaluated and released (the
    // object made for one is destroyed before the next statement).
    $o = new NoCtor(new D("arg$i"));
    echo "noctor made\n";
    echo "made $i ", count($o->p), "\n";
    $w = new WithCtor($i);
    $w2 = new WithCtor($i, "b$i");
    echo get_class(Base::make()), " ", get_class(Child::make()), "\n";
    echo Base::who(), " ", Child::who(), " ", Child::viaParent(), " ", Child::viaSelf(), "\n";
    echo (new Child)->callParentInst(), "\n";
    echo Base::nothing(1, 2), "\n";
    foreach (Base::gen() as $v) { echo "g$v "; }
    echo "\n";
    try {
        new WithCtor();
    } catch (ArgumentCountError $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
    try {
        new PrivCtor;
    } catch (Error $e) {
        echo $e->getMessage(), "\n";
    }
    echo get_class(PrivCtor::make()), "\n";
    try {
        new Abs;
    } catch (Error $e) {
        echo $e->getMessage(), "\n";
    }
    try {
        new Ifc;
    } catch (Error $e) {
        echo $e->getMessage(), "\n";
    }
    $e = new Exception("e$i");
    echo $e->getLine(), " ", $e->getMessage(), "\n";
    // A constructor that throws: the object is never destructed.
    try {
        new class(new D("held$i")) { function __construct($d) { throw new RuntimeException("ctor throws"); } function __destruct() { echo "never\n"; } };
    } catch (RuntimeException $e) {
        echo "caught ", $e->getMessage(), "\n";
    }
    echo "end $i\n";
}
echo "done\n";

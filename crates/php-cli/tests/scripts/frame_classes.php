<?php
// A frame slot keeps the classes of the last method frame at its depth
// (`Frames::call_method`): every frame pushed there after it must see its
// own scope and called class, never the slot's old ones.

class A {
    private $secret = 'a-private';
    public function __construct() {}
    public function who() { return self::class . '/' . static::class; }
    public static function make() { return static::class; }
    public function peek($o) { return $o->secret; }
}

class B extends A {
    public function mine() { return self::class . '/' . static::class; }
}

class C {
    private $secret = 'c-private';
    public function peek($o) {
        try {
            return $o->secret;
        } catch (Error $e) {
            return $e->getMessage();
        }
    }
}

function outside($o) {
    try {
        return $o->secret;
    } catch (Error $e) {
        return $e->getMessage();
    }
}

function called_here() {
    try {
        return var_export(get_called_class(), true);
    } catch (Error $e) {
        return $e->getMessage();
    }
}

$a = new A;
$b = new B;
$c = new C;

for ($i = 0; $i < 3; $i++) {
    // A method (scope A), then a function at the same depth: no scope.
    echo $a->peek($a), ' ', outside($a), "\n";
    // A static call on B (called B), then a method on $a: called is $a's class.
    echo B::make(), ' ', $a->who(), "\n";
    // An inherited method (scope A, one parent up), then B's own (scope B).
    echo $b->who(), ' ', $b->mine(), ' ', $b->peek($a), "\n";
    // Scope A, then scope C at the same depth: A's private is not C's.
    echo $a->peek($a), ' ', $c->peek($a), ' ', $c->peek($c), "\n";
    // A constructor (scope A), then a function: no scope, no called class.
    $n = new A;
    echo outside($n), ' ', called_here(), "\n";
    // Through a temporary: `(new B)->who()`.
    echo (new B)->who(), ' ', outside($c), "\n";
}

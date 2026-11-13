<?php
// The other shapes of a method call: named arguments, a spread, `?->`,
// `__call`, `__invoke`, a closure, a method named by a variable, a
// callable made from a call, a pipe.
class A {
    function m(&$a, $b = "b") { $a .= "+A"; return "A::m($a, $b)"; }
    function __call($name, $args) { $args[0] .= "+call"; return "__call $name(" . implode(", ", $args) . ")"; }
    function __invoke(&$x) { $x .= "+invoke"; return "A::__invoke"; }
    static function __callStatic($name, $args) { return "__callStatic $name"; }
}
class V {
    function m($a, $b = "b") { $a .= "+V"; return "V::m($a, $b)"; }
}
function eff($v = 1) { echo "eff "; return $v; }

function named() {
    foreach ([new A, new V, new A] as $o) {
        $x = "x"; $y = "y";
        echo $o->m(b: $y, a: $x), " x=$x\n";
        $x = "x";
        echo $o->m(a: $x), " x=$x\n";
        $x = "x";
        echo $o->m($x, b: eff("named")), " x=$x\n";
    }
    try { (new V)->m(nope: 1); } catch (Error $e) { echo $e->getMessage(), "\n"; }
}
named();

// (What a by-reference parameter writes back into the spread array is not
// printed: p8 does not write it back, for any kind of call.)
function spread() {
    foreach ([new A, new V] as $o) {
        $args = ["x", "y"];
        echo $o->m(...$args), "\n";
        $x = "x"; $rest = ["y"];
        echo $o->m($x, ...$rest), " x=$x\n";
        $by_name = ["b" => "named", "a" => "x"];
        echo $o->m(...$by_name), "\n";
    }
}
spread();

function nullsafe() {
    $n = null; $x = "x";
    var_dump($n?->m($x, eff()));
    foreach ([new A, null, new V] as $o) {
        $x = "x";
        var_dump($o?->m($x), $x);
    }
    $o = new A; $x = "x";
    var_dump($o?->m($x, $o = null), $o);
    // (A chain after a null `?->`, `$n?->m()->further()`, is not here: p8
    // does not skip the rest of the chain yet.)
    $h = new stdClass; $h->inner = null;
    var_dump($h->inner?->m(eff()));
    $h->inner = new V;
    var_dump($h->inner?->m(eff("in the property")));
}
nullsafe();

function magic() {
    $o = new A; $x = "x";
    for ($i = 0; $i < 2; $i++) {
        echo $o->undefined($x, "second"), " x=$x\n";
    }
    echo $o->other(eff("arg")), "\n";
    // __invoke by its name, and by calling the object.
    $x = "x";
    echo $o->__invoke($x), " x=$x\n";
    echo $o($x), " x=$x\n";
    // A closure's own parameters decide, one site, two closures.
    $by_ref = function (&$n, $add) { $n += $add; return $n; };
    $by_value = function ($n, $add) { $n += $add; return $n; };
    foreach ([$by_ref, $by_value, $by_ref] as $f) {
        $n = 1;
        echo $f->__invoke($n, 2), " n=$n\n";
    }
    foreach ([$by_ref, $by_value, $by_ref] as $f) {
        $n = 1;
        echo $f($n, 2), " n=$n\n";
    }
    $bound = $by_ref->bindTo(null);
    $n = 1;
    echo $bound->__invoke($n, 5), " n=$n\n";
}
magic();

function dynamic() {
    $name = "m";
    foreach ([new A, new V] as $o) {
        $x = "x";
        echo $o->$name($x), " x=$x\n";
        $x = "x";
        echo [$o, "m"]($x), " x=$x\n";
        $x = "x";
        $c = $o->m(...);
        echo $c($x), " x=$x\n";
        $x = "x";
        echo call_user_func_array([$o, "m"], [&$x]), " x=$x\n";
    }
}
dynamic();

function piped() {
    $o = new V;
    echo "x" |> $o->m(...), "\n";
    echo eff("piped") |> (new V)->m(...), "\n";
}
piped();

// A static method called on an object, and a parent's method.
class Base {
    public $log = [];
    function add(&$x) { $x .= "+Base"; return $this; }
    static function make(&$x) { $x .= "+make"; return new static; }
}
class Child extends Base {
    function add(&$x) { $x .= "+Child"; return parent::add($x); }
}
function inherited() {
    foreach ([new Base, new Child, new Base] as $o) {
        $x = "x";
        echo get_class($o->add($x)), " x=$x\n";
        $x = "x";
        echo get_class($o->make($x)), " x=$x\n";
    }
}
inherited();

// An interface's method on an enum case, a method on a class that is
// declared after the call is compiled.
interface Named { function label(string &$suffix): string; }
enum Suit: string implements Named {
    case Hearts = "H";
    case Spades = "S";
    function label(string &$suffix): string { $suffix .= "!"; return $this->name . $suffix; }
}
function enums() {
    foreach (Suit::cases() as $s) {
        $x = "?";
        echo $s->label($x), " x=$x\n";
    }
}
enums();
function late($o) { $x = "x"; return $o->m($x) . " x=$x"; }
if (true) {
    class Late { function m(&$a) { $a .= "+Late"; return "Late::m"; } }
}
echo late(new Late), "\n", late(new V), "\n", late(new Late), "\n";

<?php
// Parameter and return types naming classes, checked again and again: the
// class accepted last is remembered per type; another class is checked
// afresh (accepted when it is a subclass or implements the interface,
// refused otherwise), and the remembered one still works afterwards.
interface Shape {}
class Base implements Shape {}
class Sub extends Base {}
class Other {}
class Str { public function __toString(): string { return "str"; } }
function take(Base $b): Base { return $b; }
function shape(Shape $s): Shape { return $s; }
function maybe(?Base $b): ?Base { return $b; }
function both(Base&Shape $x) { return get_class($x); }
function ret(object $o): Base { return $o; }
function str(string $s) { return $s; }
function many(Base $a, Shape $b, Other $c) { return get_class($a) . get_class($b) . get_class($c); }
$objs = [new Base, new Sub, new Other, new Base, new Sub, null, new Base];
for ($round = 0; $round < 2; $round++) {
    foreach ($objs as $o) {
        foreach (['take', 'shape', 'maybe', 'both', 'ret'] as $f) {
            try {
                $r = $f($o);
                echo $f, "(", $o === null ? 'null' : get_class($o), ") = ", is_object($r) ? get_class($r) : var_export($r, true), "\n";
            } catch (TypeError $e) {
                echo $f, ": ", $e->getMessage(), "\n";
            }
        }
    }
    echo str(new Str), "\n";
    try { echo many(new Sub, new Base, new Other), "\n"; echo many(new Base, new Sub, new Sub), "\n"; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
    $fn = function (Base $b): Shape { return $b; };
    foreach ([new Base, new Sub, new Other] as $o) {
        try { echo get_class($fn($o)), "\n"; } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
    }
}

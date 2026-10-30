<?php
interface I { function m(); }
trait T { function t() {} }
abstract class A implements I { function m() {} function own() {} private function priv() {} abstract function __construct(); function __toString(): string { return ''; } }
class B extends A { function m() {} function own() {} function priv() {} function __construct() {} function extra() {} }
class C extends B { function m() {} function __construct() {} use T; }
class D implements Countable { function count(): int { return 0; } }
foreach ([[B::class, 'm'], [C::class, 'm'], [B::class, 'own'], [B::class, 'priv'], [B::class, '__construct'], [C::class, '__construct'], [B::class, 'extra'], [A::class, 'm'], [D::class, 'count'], [C::class, 't'], [B::class, '__toString']] as [$c, $m]) {
    $r = new ReflectionMethod($c, $m);
    try {
        $p = $r->getPrototype();
        echo "$c::$m -> ", $p->class, '::', $p->name, ' ', var_export($r->hasPrototype(), true), "\n";
    } catch (ReflectionException $e) {
        echo "$c::$m: ", $e->getMessage(), ' ', var_export($r->hasPrototype(), true), "\n";
    }
}
var_dump(class_uses(new C), class_uses(B::class), @class_uses('Nope', false));

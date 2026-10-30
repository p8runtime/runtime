<?php
namespace N;
interface I {}
abstract class B { public function inh(): int { return 1; } abstract protected function ab(int $x): void; }
class C extends B implements I {
    /** doc */
    public function m(int $a, ?string $b = null, array &$c = [], ...$rest): ?array { return null; }
    protected function ab(int $x): void {}
    final public static function s(): static { return new static; }
    private function p() {}
    public function __construct() {}
}
foreach (['m', 'ab', 's', 'p', '__construct', 'inh'] as $n) echo new \ReflectionMethod(C::class, $n), "---\n";
echo new \ReflectionMethod(B::class, 'ab'), "---\n";
function f(int $a = 1) { static $s; return $a; }
echo new \ReflectionFunction('N\f'), "---\n";
$a = 1; $cl = function ($x) use ($a) { static $st = 2; }; echo new \ReflectionFunction($cl), "---\n";
echo new \ReflectionMethod(C::class, 'ab'), "---\n";
class D extends C { function &r() { static $x; return $x; } }
echo new \ReflectionMethod(D::class, 'r');

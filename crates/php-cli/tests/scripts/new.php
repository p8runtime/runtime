<?php
namespace { 
class A { function __construct($x) { echo "A ctor\n"; } }
class B { function __construct() { echo "B ctor\n"; } }
class C {}
function f() { echo "f called\n"; return 1; }
var_dump(new A(new B));
var_dump(new C(f()));
class P { private function __construct() {} }
try { new P(f()); } catch (Error $e) { echo $e->getMessage(), "\n"; }
abstract class Q {}
try { new Q(f()); } catch (Error $e) { echo $e->getMessage(), "\n"; }
$e = new Exception('m', 3); echo $e->getCode(), $e->getLine(), "\n";
class D { public $v; function __construct(...$a) { $this->v = $a; } }
var_dump(new D(1, ...[2, 3]));
$cls = 'B'; $x = new $cls; var_dump($x);
var_dump(new class(5) { function __construct(public $n) {} });
interface I2 { function hi(); }
class P2 { function __construct(public $p = 'pp') {} }
$o = new class(5) { function __construct(public $n) {} };
var_dump($o);
$f = function () { return new class extends P2 implements I2 { function hi() { return __CLASS__ === static::class; } }; };
$a = $f(); $b = $f();
var_dump(get_class($a) === get_class($b), $a instanceof P2, $a instanceof I2, $a->hi(), $a);
echo strlen(get_class($o)) > 20 ? "long\n" : "short\n";
echo get_debug_type($o), ' ', get_debug_type($a), "\n";
print_r($o); echo "\n";
var_dump((new class { const X = 3; public function m() { return self::X; } })->m());
class Q2 { function make() { return new class { function who() { return __METHOD__; } }; } }
var_dump(str_starts_with((new Q2)->make()->who(), "class@anonymous"));
try { $o->nope(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
$args = [1, 2]; $s = new class(...$args) { public $a; function __construct(...$x) { $this->a = $x; } }; var_dump($s->a);
}
namespace N {
class B { const K = 'k'; public static $s = 1; function __construct() { echo "B ctor\n"; } static function make() { return static::class; } }
class C extends B {}
$cls = 'N\B'; $x = new $cls; var_dump($x);
$o = new C;
var_dump($o::K, $o::make(), $cls::make(), $cls::$s, $o::$s);
$cls::$s = 5; var_dump(B::$s);
$name = '\N\C'; var_dump(new $name instanceof B);
var_dump($o instanceof $cls, $x instanceof $o, 5 instanceof $cls);
$arr = ['c' => 'N\C']; var_dump(get_class(new $arr['c']));
var_dump(new ('N' . '\B'));
try { $n = 5; new $n; } catch (\Error $e) { echo $e->getMessage(), "\n"; }
try { $n = 'Nope'; new $n; } catch (\Error $e) { echo $e->getMessage(), "\n"; }
try { $n = []; $n::f(); } catch (\Error $e) { echo $e->getMessage(), "\n"; }
}

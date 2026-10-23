<?php
const FOO = 'foo';
define('BAR', 10);
class A {
    const X = 2;
    const Y = self::X * 3;
    const Z = 'z' . PHP_EOL;
    const ARR = [self::X => 'two', 'k' => FOO];
    const SPREAD = [0, ...self::LIST, 4];
    const LIST = [1, 2, 3];
    public $p = FOO;
    public $q = self::Y + BAR;
    public int $max = PHP_INT_MAX;
    public static $s = self::X . '-static';
    public $cls = self::class;
    public $magic = __CLASS__;
}
class B extends A {
    const X = 5;
    const W = parent::X + self::X;
    public $r = A::ARR['k'];
}
var_dump(A::Y, A::Z, A::ARR, A::SPREAD, B::W, B::Y);
var_dump(new A);
var_dump(A::$s, B::$s);
B::$s = 'changed'; var_dump(A::$s);
var_dump((new B)->r, (new B)->q);
enum E: int { case One = 1 << 0; case Two = 1 << 1; const ALIAS = self::Two; }
var_dump(E::Two->value, E::ALIAS);
class C { const BAD = UNDEFINED_THING; public $ok = 1; }
var_dump(new C);
try { var_dump(C::BAD); } catch (Error $e) { echo get_class($e), ': ', $e->getMessage(), ' line ', $e->getLine(), "\n"; }
class D { const SELF = self::SELF; }
try { var_dump(D::SELF); } catch (Error $e) { echo $e->getMessage(), "\n"; }
class F { public $x = NOPE; }
try { new F; } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { new F; } catch (Error $e) { echo "again: ", $e->getMessage(), "\n"; }
interface I { const V = A::X + 1; }
class G implements I { public $v = self::V; }
var_dump((new G)->v, G::V);

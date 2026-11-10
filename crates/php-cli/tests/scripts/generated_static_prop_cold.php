<?php
// FETCH_STATIC_PROP (ops.def) and its cold cases: the first read of
// each, `static::` and a trait's `self::` for several classes in turn
// (the cache entry replaced), inherited and redeclared properties,
// references, private and protected ones from inside and outside, a
// typed one, initializers run on first use, undeclared ones.
// No `@`; the expected output is PHP 8.6's.

class A {
    public static $n = 'A';
    public static $count = 0;
    public static $list = [1, 2];
    private static $secret = 'a-secret';
    protected static $prot = 'a-prot';
    public static int $typed;
    public static function names() { return [self::$n, static::$n, A::$n]; }
    public static function bump() { static::$count++; return static::$count; }
    public static function secret() { return self::$secret; }
    public static function prot() { return static::$prot; }
    public static function typed() { return static::$typed; }
}
class B extends A { public static $n = 'B'; protected static $prot = 'b-prot'; }
class C2 extends B {}

echo "-- named, self, static\n";
for ($i = 0; $i < 2; $i++) {
    echo json_encode(A::names()), json_encode(B::names()), json_encode(C2::names()), "\n";
}
// An inherited property is one cell: written through any class.
echo A::bump(), B::bump(), C2::bump(), A::$count, "\n";
// An array: the copy changed, the property not.
function lists() { $l = A::$list; $l[] = 3; return [$l, A::$list]; }
echo json_encode(lists()), json_encode(lists()), "\n";
// A reference bound into the property, then changed through it.
$x = 'ref1';
A::$n = &$x;
echo json_encode(A::names()), "\n";
$x = 'ref2';
echo json_encode(A::names()), json_encode(B::names()), "\n";
A::$n = 'A';

echo "-- visibility\n";
for ($i = 0; $i < 2; $i++) {
    echo A::secret(), " ", B::secret(), " ", A::prot(), " ", B::prot(), " ", C2::prot(), "\n";
}
try { echo A::$secret; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { echo B::$prot; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
// A closure bound to A, then to an unrelated scope: the private one.
$f = function () { return A::$secret; };
echo Closure::bind($f, null, A::class)(), "\n";
try { echo Closure::bind($f, null, B::class)(), "\n"; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
echo Closure::bind($f, null, A::class)(), "\n";

echo "-- typed, undeclared\n";
A::$typed = 5;
var_dump(A::typed(), A::typed());
function undeclared() { return A::$nope; }
for ($i = 0; $i < 2; $i++) {
    try { undeclared(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}

echo "-- initializers, traits\n";
class Lazy {
    const BASE = 10;
    public static $v = self::BASE * 2;
    public static function get() { return self::$v; }
}
var_dump(Lazy::get(), Lazy::get());
trait T {
    public static $tv = 't';
    public static function mine() { return [self::$tv, static::$tv]; }
}
class UT { use T; }
class UT2 extends UT {}
class UT3 { use T; }
UT3::$tv = 'ut3';
for ($i = 0; $i < 2; $i++) { echo json_encode(UT::mine()), json_encode(UT2::mine()), json_encode(UT3::mine()), "\n"; }
// `static::` with no called class.
try {
    $g = function () { return static::$n; };
    $g();
} catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
// The dynamic forms.
$cls = 'B';
$obj = new C2;
echo $cls::$n, " ", $obj::$n, "\n";

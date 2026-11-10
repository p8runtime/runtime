<?php
// CLASS_CONST (ops.def) and its cold cases: the first read of each,
// `static::` and a trait's `self::` for several classes in turn (the
// cache entry replaced), private and protected constants, enum cases,
// constants computed from code, `::class`, an interface's, a missing one.
// No `@`; the expected output is PHP 8.6's.

interface HasKind { const KIND = 'iface'; }
class A implements HasKind {
    const NAME = 'A';
    const LIST = [1, 2];
    const COMPUTED = self::NAME . '-' . PHP_INT_SIZE;
    private const SECRET = 'a-secret';
    protected const PROT = 'a-prot';
    public static function names() { return [self::NAME, static::NAME, static::class, self::class]; }
    public static function secret() { return self::SECRET; }
    public static function prot() { return static::PROT; }
}
class B extends A {
    const NAME = 'B';
    protected const PROT = 'b-prot';
    public static function up() { return [parent::NAME, parent::class, self::KIND]; }
}
class C2 extends B { const NAME = 'C2'; }

echo "-- named\n";
function named() { return [A::NAME, B::NAME, A::KIND, HasKind::KIND, A::COMPUTED, A::class, B::class]; }
for ($i = 0; $i < 2; $i++) { echo json_encode(named()), "\n"; }
// An array constant: the copy is changed, the constant is not.
function lists() { $l = A::LIST; $l[] = 3; return [$l, A::LIST]; }
echo json_encode(lists()), json_encode(lists()), "\n";

echo "-- self, static, parent\n";
// Called through A, B, C2 in turn, twice: `static::` reads each class's.
for ($i = 0; $i < 2; $i++) {
    echo json_encode(A::names()), json_encode(B::names()), json_encode(C2::names()), json_encode(B::up()), "\n";
}
// Private and protected: read from inside, an error from outside.
echo A::secret(), " ", B::secret(), " ", A::prot(), " ", B::prot(), " ", C2::prot(), "\n";
try { echo A::SECRET; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { echo B::PROT; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }

echo "-- traits\n";
trait T {
    const TC = 'trait-const';
    public static function mine() { return [self::TC, static::TC, self::class, static::class]; }
}
class UT { use T; }
class UT2 extends UT { const TC = 'ut2'; }
class UT3 { use T; }
for ($i = 0; $i < 2; $i++) { echo json_encode(UT::mine()), json_encode(UT2::mine()), json_encode(UT3::mine()), "\n"; }
try { echo T::TC; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }

echo "-- enums\n";
enum Suit: string {
    case Hearts = 'H';
    case Spades = 'S';
    const Wild = self::Spades;
    public static function first() { return [self::Hearts, static::Spades->value, self::Wild]; }
}
function suits() { return [Suit::Hearts, Suit::Hearts === Suit::Hearts, Suit::Wild->name]; }
var_dump(suits(), suits()[0] === Suit::first()[0]);
var_dump(Suit::first());

echo "-- errors\n";
function missing() { return A::NOPE; }
for ($i = 0; $i < 2; $i++) {
    try { missing(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}
// A constant whose expression throws: an error each time, never cached.
class Bad { const X = 1 % 0; }
for ($i = 0; $i < 2; $i++) {
    try { echo Bad::X; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}
// `static::` with no called class.
try {
    $f = function () { return static::NAME; };
    $f();
} catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
// A closure bound to one class, then another: its `static::` follows.
$g = function () { return static::NAME; };
echo Closure::bind($g, null, A::class)(), Closure::bind($g, null, B::class)(), Closure::bind($g, null, A::class)(), "\n";
// The dynamic forms.
$cls = 'B';
$obj = new C2;
echo $cls::NAME, " ", $obj::NAME, " ", $obj::class, "\n";

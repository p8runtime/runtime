<?php
// Class constants read again and again (the cached read): public ones of a
// named class and `::class`; and what must not be cached or must still
// fail: the same name on another class, protected/private ones from
// scopes that may and may not see them, enum cases, constants whose value
// is code, self::/static:: in inherited code, an undefined constant.
class A {
    public const X = 'A-x';
    const LIST = [1, 2, 3];
    protected const P = 'A-p';
    private const Q = 'A-q';
    const E = self::X . '+' . B::Y;
    public static function show() { return [self::X, static::X, self::P, self::Q]; }
    public function inside() { return A::P . A::Q; }
}
class B extends A {
    const X = 'B-x';
    const Y = 'B-y';
    public function child() { return A::P; }
    public function childQ() { return A::Q; }
}
enum Suit: string { case Hearts = 'H'; case Spades = 'S'; const Wild = self::Spades; }
function reads() {
    $out = [A::X, B::X, A::LIST, A::E, B::E, A::class, B::class, Suit::Hearts->value, Suit::Wild->name];
    $l = A::LIST;
    $l[] = 4;
    $out[] = count(A::LIST);
    $out[] = Suit::Hearts === Suit::Hearts;
    return $out;
}
for ($i = 0; $i < 3; $i++) {
    echo "-- $i\n";
    echo json_encode(reads()), "\n";
    echo json_encode(A::show()), json_encode(B::show()), "\n";
    echo (new A)->inside(), (new B)->child(), "\n";
    try { echo (new B)->childQ(), "\n"; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    try { echo A::P, "\n"; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    try { echo A::NOPE, "\n"; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    $f = function () { return A::P; };
    try { echo $f(), "\n"; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
    $g = Closure::bind($f, null, A::class);
    echo $g(), "\n";
    try { echo $f(), "\n"; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}

<?php
// `new` of a class whose constructor is private or protected: from a
// scope that may call it (made in the loop), and from one that may not
// (the error, the object never constructed nor destructed). No `@`; the
// expected output is PHP 8.6's.

class Point {
    private function __construct(public int $x, public int $y) { echo "ctor($x,$y) "; }
    public function __destruct() { echo "~{$this->x} "; }
    public static function at(int $x, int $y): self { return new self($x, $y); }
    public static function origin(): static { return new static(0, 0); }
}
class Base {
    protected function __construct(public string $n = 'base') {}
    public static function make(): static { return new static(); }
}
class Child extends Base {
    public static function child(): self { return new self('child'); }
    public static function parentOne(): Base { return new parent('via-parent'); }
}
class NoCtor {}

for ($i = 0; $i < 3; $i++) {
    $p = Point::at($i, $i + 1);
    echo $p->x + $p->y, "\n";
}
unset($p);
echo "\n";
var_dump(Point::origin()->x);
echo Base::make()->n, " ", Child::make()->n, " ", Child::child()->n, " ", Child::parentOne()->n, "\n";
// From outside: the error, for each kind of scope.
function outside() { return new Point(1, 2); }
for ($i = 0; $i < 2; $i++) {
    try { outside(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
}
try { new Base(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
// A class unrelated to Base calling its protected constructor.
class Other { public static function try() { return new Base(); } }
try { Other::try(); } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
// A closure bound to the class may call it.
$f = Closure::bind(fn() => new Point(7, 8), null, Point::class);
echo $f()->x, "\n";
var_dump(new NoCtor instanceof NoCtor);
echo "end\n";

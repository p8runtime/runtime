<?php
// The forms perf/objects.php runs, and their neighbours: an int passed,
// returned or stored where a float is declared; `static` returned; float
// arithmetic on properties. Each answer as PHP 8.6 gives it.

class Point {
    public function __construct(private float $x = 0.0, private float $y = 0.0) {}
    public function x(): float { return $this->x; }
    public function y(): float { return $this->y; }
    public function add(Point $o): static { $this->x += $o->x(); $this->y += $o->y(); return $this; }
    public function sub(Point $o): static { $this->x -= $o->x(); $this->y -= $o->y(); return $this; }
    public function scale(float $k): static { $this->x *= $k; $this->y *= $k; return $this; }
    public function div(float $k): static { $this->x /= $k; $this->y /= $k; return $this; }
    public function pow(float $k): static { $this->x **= $k; $this->y **= $k; return $this; }
    public function parent(): static { return new Point($this->x, $this->y); }
    public function wrongInt(): int { return $this; }
    public function wrongPoint(): Point { return $this; }
    public static function make(int $x): static { return new static($x, $x); }
}
class Point3 extends Point {}

// The loop itself (as perf/objects.php, shorter).
$sum = new Point();
for ($i = 0; $i < 1000; $i++) {
    $sum->add(new Point($i % 7, $i % 11));
}
var_dump($sum->x(), $sum->y(), round($sum->x() + $sum->y()));

// Floats made from ints at entry: positional, named, defaults, mixed
// with a class-typed argument, huge and negative ints.
$p = new Point(3, -4);
var_dump($p->x(), $p->y());
$p = new Point(y: 2);
var_dump($p->x(), $p->y());
$p = new Point(PHP_INT_MAX, -9007199254740993);
var_dump($p->x(), $p->y());
var_dump((new Point(1, 2))->add(new Point(10, 20))->x());

// The arithmetic on float properties, each operator.
$p = new Point(1.5, -2.25);
var_dump($p->add(new Point(0.25, 0.25))->x(), $p->sub(new Point(1, 1))->y());
var_dump($p->scale(4)->x(), $p->scale(-1)->y());
var_dump($p->div(2)->x(), $p->pow(2)->y());
try {
    $p->div(0);
} catch (DivisionByZeroError $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
// (What the failed `/=` leaves in x is not printed: PHP 8.6 leaves 0,
// p8 the value before; not this test's.)
var_dump($p->y());
$q = new Point(INF, NAN);
var_dump($q->add(new Point(-1, 1))->x(), $q->y(), (new Point(0, 1))->scale(-1)->x());

// `static` returned: the called class's object, a subclass's, the
// errors of a parent's object and of $this for other types.
$r = new Point3(1, 2);
var_dump(get_class($r->add(new Point(1, 1))), get_class(Point3::make(5)), get_class(Point::make(6)));
foreach (['parent', 'wrongInt'] as $m) {
    try {
        $r->$m();
        echo "$m: no error\n";
    } catch (TypeError $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
var_dump(get_class($r->wrongPoint()), get_class((new Point)->parent()));

// Ints for floats in other declarations: unions, nullable, by
// reference, variadic, return, property; and what is not a float's.
function f_union(float|string $x) { var_dump($x); }
function f_bool(float|bool $x) { var_dump($x); }
function f_nullable(?float $x) { var_dump($x); }
function f_num(int|float $x) { var_dump($x); }
function f_strbool(string|bool $x) { var_dump($x); }
function f_ref(float &$x) { var_dump($x); $x += 0.5; }
function f_var(float ...$xs) { var_dump($xs); }
function f_mixed(Point $p, float $k, float $j) { var_dump($p->x() * $k + $j); }
function f_ret(int $i): float { return $i; }
function f_ret_static_none(): ?float { return null; }
f_union(5); f_union("5"); f_bool(7); f_bool(false); f_nullable(8); f_nullable(null);
f_num(9); f_strbool(5);
$v = 3; f_ref($v); var_dump($v);
f_var(1, 2.5, 3); f_var();
f_mixed(new Point(2, 0), 3, 1);
var_dump(f_ret(4), f_ret_static_none());
class Holder { public float $f = 0; public ?float $g = null; public int|float $h = 0; }
$h = new Holder; $h->f = 11; $h->g = 12; $h->h = 13;
var_dump($h);
$h->f += 1; $h->f -= 0.5; $h->f *= 2;
var_dump($h->f);
try {
    f_mixed(new Point, "x", 1);
} catch (TypeError $e) {
    echo get_class($e), ": ", preg_replace('/, called in .*/', '', $e->getMessage()), "\n";
}
try {
    f_mixed(new Point, 2, []);
} catch (TypeError $e) {
    echo get_class($e), ": ", preg_replace('/, called in .*/', '', $e->getMessage()), "\n";
}
function f_arrvar(float $k, array ...$xs) { var_dump($k, $xs); }
f_arrvar(1, [2]);
try {
    f_arrvar(1, 2);
} catch (TypeError $e) {
    echo get_class($e), ": ", preg_replace('/, called in .*/', '', $e->getMessage()), "\n";
}
try {
    f_mixed(new Holder, 2, 3);
} catch (TypeError $e) {
    echo get_class($e), ": ", preg_replace('/, called in .*/', '', $e->getMessage()), "\n";
}

// Float operators outside properties: an element, a static property.
class S { public static float $s = 1.5; }
S::$s += 2.25; S::$s *= 2; S::$s -= 1;
var_dump(S::$s);
$a = ['k' => 1.5];
$a['k'] += 0.75; $a['k'] *= 4; $a['k'] -= 10;
var_dump($a);

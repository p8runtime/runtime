<?php
// `$o->p += y`, `-=` and `*=` with a float in the property and a float
// operand: computed in the property's slot when the property's type takes
// the float (untyped, float, ?float, int|float, mixed); a type that does not
// (int, string, bool) and every other operator and operand go where they
// went. Each site runs more than once: the property cache answers from the
// second.
class U { public $p = 0.5; }
class F { public float $p = 0.5; }
class NF { public ?float $p = 0.5; }
class IF_ { public int|float $p = 0.5; }
class MX { public mixed $p = 0.5; }
class I { public int $p = 1; }
class S { public string $p = '1.5'; }
class B { public bool $p = true; }

function add($o, $y) { return $o->p += $y; }
function sub($o, $y) { return $o->p -= $y; }
function mul($o, $y) { return $o->p *= $y; }
function div($o, $y) { return $o->p /= $y; }
function pow_($o, $y) { return $o->p **= $y; }
function mod($o, $y) { return $o->p %= $y; }
function cat($o, $y) { return $o->p .= $y; }

function show($what, $f) {
    try {
        $r = $f();
        echo $what, ': ';
        var_dump($r);
    } catch (Throwable $e) {
        echo $what, ': ', get_class($e), ': ', $e->getMessage(), "\n";
    }
}

echo "-- floats into types that take a float\n";
foreach (['U', 'F', 'NF', 'IF_', 'MX'] as $c) {
    echo "$c\n";
    for ($i = 0; $i < 3; $i++) {
        $o = new $c;
        var_dump(add($o, 1.25), sub($o, 4.5), mul($o, -2.0), $o->p);
        var_dump(add($o, 0.1), add($o, 0.2), $o->p);
        var_dump(mul($o, 1e308), mul($o, 10.0), sub($o, INF), $o->p);
        $o->p = -0.0;
        var_dump(mul($o, 1.0), add($o, -0.0), add($o, 0.0), $o->p);
        $o->p = NAN;
        var_dump(add($o, 1.0), $o->p);
    }
}

echo "-- operands and values that are not two floats\n";
foreach (['U', 'F', 'IF_'] as $c) {
    echo "$c\n";
    for ($i = 0; $i < 3; $i++) {
        $o = new $c;
        show('float + int', fn() => add($o, 2));
        show('float + "2.5"', fn() => add($o, '2.5'));
        show('float + true', fn() => add($o, true));
        show('float + null', fn() => sub($o, null));
        show('float + "x"', fn() => add($o, 'x'));
        if ($c !== 'F') {
            $o->p = 3;
            show('int + float', fn() => add($o, 0.5));
            $o->p = '1.5';
            show('"1.5" * float', fn() => mul($o, 2.0));
        }
        $o->p = 1.5;
        var_dump($o->p);
    }
}

echo "-- types that do not take the float\n";
// (The property afterwards: where the type converts the result, p8's value
// of the expression is not PHP's, #489.)
function quiet($what, $f) {
    try {
        $f();
        echo $what, "\n";
    } catch (Throwable $e) {
        echo $what, ': ', get_class($e), ': ', $e->getMessage(), "\n";
    }
}
for ($i = 0; $i < 3; $i++) {
    $o = new I;
    quiet('int + 1.5', fn() => add($o, 1.5));
    var_dump($o->p);
    quiet('int + 2.0', fn() => add($o, 2.0));
    var_dump($o->p);
    $s = new S;
    quiet('string + 1.0', fn() => add($s, 1.0));
    var_dump($s->p);
    $b = new B;
    quiet('bool * 0.0', fn() => mul($b, 0.0));
    var_dump($b->p);
}

echo "-- an int property written with floats by every way there is, then op=\n";
class IW { public int $p = 0; public ?int $q = null; }
function after($what, $o) {
    quiet("$what +=", fn() => add($o, 1.5));
    var_dump($o->p);
}
for ($i = 0; $i < 3; $i++) {
    $o = new IW;
    $o->p = 2.0;
    after('assigned 2.0', $o);
    $o = new IW;
    [$o->p] = [3.0];
    after('destructured', $o);
    $o = new IW;
    foreach ([4.0] as $o->p) {}
    after('foreach', $o);
    $o = new IW;
    (new ReflectionProperty(IW::class, 'p'))->setValue($o, 5.0);
    after('reflection', $o);
    $o = new IW;
    $r = &$o->p;
    $r = 7.0;
    unset($r);
    after('through a reference', $o);
    $o = new IW;
    $o->p = 8;
    $o->p++;
    $o->p /= 3;
    after('/=', $o);
    $o = new IW;
    $o->p = PHP_INT_MAX;
    quiet('overflow', fn() => add($o, 1));
    after('overflowed', $o);
    $o = new IW;
    (function () { $this->p = 9.0; })->call($o);
    after('closure', $o);
}

echo "-- the other operators on two floats\n";
// (A fresh object after the division by zero: what a failed operator leaves
// in a typed property is #438's; the value of a converted `%` or `.` on a
// float property is #489's, so only the property is printed.)
foreach (['U', 'F'] as $c) {
    for ($i = 0; $i < 3; $i++) {
        $o = new $c;
        show("$c / 2.0", fn() => div($o, 2.0));
        show("$c / 0.0", fn() => div($o, 0.0));
        $o = new $c;
        show("$c ** 2.0", fn() => pow_($o, 2.0));
        $o->p = 7.5;
        quiet("$c % 2.0", fn() => mod($o, 2.0));
        var_dump($o->p);
        quiet("$c . 2.5", fn() => cat($o, 2.5));
        var_dump($o->p);
    }
}

echo "-- readonly, set visibility, a reference, hooks, magic, unset\n";
class RO {
    public function __construct(public readonly float $p) {}
    public function bump() { return $this->p += 1.0; }
}
class PS {
    public private(set) float $p = 1.0;
    public function bump() { return $this->p += 1.5; }
}
class H {
    public float $p = 1.0 {
        get { echo "get\n"; return $this->p; }
        set { echo "set $value\n"; $this->p = $value; }
    }
}
class M {
    public $p = 1.0;
    public function __get($n) { echo "__get $n\n"; return 2.5; }
    public function __set($n, $v) { echo "__set $n\n"; }
}
for ($i = 0; $i < 3; $i++) {
    $r = new RO(1.0);
    show('readonly inside', fn() => $r->bump());
    show('readonly outside', fn() => add($r, 1.0));
    $s = new PS;
    show('set visibility inside', fn() => $s->bump());
    show('set visibility outside', fn() => add($s, 1.0));
    $u = new U;
    $ref = &$u->p;
    show('reference', fn() => add($u, 1.5));
    var_dump($ref);
    unset($ref);
    $t = new F;
    $ref = &$t->p;
    show('typed reference', fn() => mul($t, 3.0));
    var_dump($ref);
    unset($ref);
    $h = new H;
    show('hooks', fn() => add($h, 0.5));
    $m = new M;
    show('magic', fn() => add($m, 0.5));
    unset($m->p);
    show('magic unset', fn() => add($m, 0.5));
    $f = new F;
    unset($f->p);
    show('typed unset', fn() => add($f, 0.5));
}

echo "-- one site, classes in turn: its entry filled again for each\n";
class M2 {
    public $p = 1.0;
    public function __get($n) { echo "__get $n\n"; return 2.5; }
    public function __set($n, $v) { echo "__set $n $v\n"; }
}
class M3 extends M2 {}
for ($i = 0; $i < 3; $i++) {
    $u = new U;
    $m = new M2;
    $n = new M3;
    unset($n->p);
    var_dump(add($u, 1.0), add($m, 1.0), add($n, 1.0), add($u, 1.0), add($n, 1.0), add($m, 1.0));
    $l = (new ReflectionClass(F::class))->newLazyGhost(function ($o) { echo "init\n"; $o->p = 4.0; });
    var_dump(add(new F, 1.0), add($l, 1.0), add($l, 1.0));
}

echo "-- \$this, a constant operand, and the released value\n";
class K {
    public float $x = 0.0;
    public $y = 0.0;
    public function run() { $this->x += 1.5; $this->x *= 3.0; $this->x -= 0.25; $this->y += $this->x; return [$this->x, $this->y]; }
}
class D { public function __destruct() { echo "destruct\n"; } }
for ($i = 0; $i < 3; $i++) {
    $k = new K;
    var_dump($k->run(), $k->run());
    $u = new U;
    $x = new D;
    echo "before\n";
    $x = ($u->p += 1.0);
    echo "after\n";
    var_dump($x);
}

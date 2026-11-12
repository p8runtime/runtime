<?php
// `$o->p += y`, `-=` and `*=` on a declared property (#413): two integers that
// do not overflow are computed in the property's slot. An overflow is a
// float exactly where PHP's is, and every other case (a type that does not
// take the result, readonly, a narrower set visibility, a reference, hooks,
// magic, an unset slot, an operand that is not an int) goes where it went.
// Each site runs more than once: the property cache answers from the second.
class U { public $p = 0; }
class T { public int $p = 0; }
class N { public ?int $p = 0; }
class IntStr { public int|string $p = 0; }
class IntFloat { public int|float $p = 0; }
class F { public float $p = 0.0; }
class FD { public float $p = 1; }

function add($o, $y) { return $o->p += $y; }
function sub($o, $y) { return $o->p -= $y; }
function mul($o, $y) { return $o->p *= $y; }
function div($o, $y) { return $o->p /= $y; }
function mod($o, $y) { return $o->p %= $y; }
function pow_($o, $y) { return $o->p **= $y; }
function cat($o, $y) { return $o->p .= $y; }
function shl($o, $y) { return $o->p <<= $y; }
function bor($o, $y) { return $o->p |= $y; }
function show($what, $f) {
    try {
        $r = $f();
        echo $what, ': ';
        var_dump($r);
    } catch (Throwable $e) {
        echo $what, ': ', get_class($e), ': ', $e->getMessage(), "\n";
    }
}
// What the operator gives on two plain values: [it did not throw, its result].
function plain($f, $x, $y) {
    try {
        return [true, @match ($f) {
            'add' => $x + $y, 'sub' => $x - $y, 'mul' => $x * $y,
            'div' => $x / $y, 'mod' => $x % $y, 'pow_' => $x ** $y,
            'cat' => $x . $y, 'shl' => $x << $y, 'bor' => $x | $y,
        }];
    } catch (Throwable $e) {
        return [false, null];
    }
}
// `$f($o, $y)`: what it throws; its value where the property's type takes the
// operator's result as it is (where the type converts it, p8's value of the
// expression is not PHP's: #489); and the property afterwards (but not of a
// typed property whose operator threw: #438).
function op($what, $o, $f, $y) {
    [$ok, $plain] = plain($f, $o->p, $y);
    $as_it_is = $o instanceof U || is_int($plain) || ($o instanceof IntFloat && is_float($plain));
    try {
        $r = $f($o, $y);
        echo $what, ': ';
        if ($as_it_is) {
            var_dump($r);
        } else {
            echo "converted\n";
        }
    } catch (Throwable $e) {
        echo $what, ': ', get_class($e), ': ', $e->getMessage(), "\n";
    }
    if ($ok || $o instanceof U) {
        echo '  left: ';
        var_dump($o->p);
    }
}

echo "-- no overflow\n";
foreach (['U', 'T', 'N', 'IntStr', 'IntFloat'] as $c) {
    $o = new $c;
    $o->p = 5;
    var_dump(add($o, 3), sub($o, 10), mul($o, -4), $o->p);
}

echo "-- the last integer before an overflow, and the first after\n";
foreach (['U', 'T', 'N', 'IntStr', 'IntFloat'] as $c) {
    echo "$c\n";
    foreach ([
        'max-1 + 1' => [PHP_INT_MAX - 1, 'add', 1],
        'max + 1' => [PHP_INT_MAX, 'add', 1],
        'max + 0' => [PHP_INT_MAX, 'add', 0],
        'max + -1' => [PHP_INT_MAX, 'add', -1],
        'min + -1' => [PHP_INT_MIN, 'add', -1],
        'min+1 - 1' => [PHP_INT_MIN + 1, 'sub', 1],
        'min - 1' => [PHP_INT_MIN, 'sub', 1],
        '0 - min' => [0, 'sub', PHP_INT_MIN],
        '-1 - min' => [-1, 'sub', PHP_INT_MIN],
        'max - -1' => [PHP_INT_MAX, 'sub', -1],
        'max * 1' => [PHP_INT_MAX, 'mul', 1],
        'max * 2' => [PHP_INT_MAX, 'mul', 2],
        'max * -1' => [PHP_INT_MAX, 'mul', -1],
        'min * -1' => [PHP_INT_MIN, 'mul', -1],
        '-1 * min' => [-1, 'mul', PHP_INT_MIN],
        'min * 0' => [PHP_INT_MIN, 'mul', 0],
        '3037000500 * 3037000500' => [3037000500, 'mul', 3037000500],
        '3037000499 * 3037000499' => [3037000499, 'mul', 3037000499],
        '4294967296 * -2147483648' => [4294967296, 'mul', -2147483648],
        '4294967296 * 2147483648' => [4294967296, 'mul', 2147483648],
    ] as $what => [$start, $f, $y]) {
        $o = new $c;
        $o->p = $start;
        op($what, $o, $f, $y);
    }
}

echo "-- operands that are not integers\n";
class AA implements ArrayAccess {
    function offsetExists($k): bool { return true; }
    function offsetGet($k): mixed { return 1; }
    function offsetSet($k, $v): void {}
    function offsetUnset($k): void {}
}
foreach (['U', 'T'] as $c) {
    echo "$c\n";
    foreach ([1.5, 2.0, '5', '5.5', ' 5', '5 ', '5 apples', 'abc', '', true, false, null, [], [1], new stdClass, new AA] as $y) {
        foreach (['add', 'sub', 'mul'] as $f) {
            $o = new $c;
            $o->p = 7;
            op($f . ' ' . (is_object($y) ? get_class($y) : json_encode($y)), $o, $f, $y);
        }
    }
}

echo "-- a float property takes an integer as a float\n";
for ($i = 0; $i < 3; $i++) {
    $f = new F;
    $f->p = 1.5;
    add($f, 2);
    var_dump($f->p);
    mul($f, 2);
    var_dump($f->p);
    sub($f, 7);
    var_dump($f->p);
    // (Its default is an integer in p8's slot, #489: the result of two
    // integers is still the property's type's to take.)
    $d = new FD;
    add($d, 2);
    var_dump($d->p);
    $d = new FD;
    mul($d, 3);
    var_dump($d->p);
    sub($d, 3);
    var_dump($d->p);
}

echo "-- readonly\n";
class RO {
    public function __construct(public readonly int $p) {}
    public function bump() { return $this->p += 1; }
}
for ($i = 0; $i < 3; $i++) {
    $r = new RO(1);
    show('inside', fn() => $r->bump());
    show('outside', fn() => add($r, 1));
    show('outside *', fn() => mul($r, 2));
    var_dump($r->p);
}

echo "-- a narrower set visibility\n";
class PS {
    public private(set) int $p = 1;
    public function bump() { return $this->p += 1; }
}
class PS2 extends PS {
    public function child() { return $this->p -= 1; }
}
for ($i = 0; $i < 3; $i++) {
    $s = new PS2;
    show('inside', fn() => $s->bump());
    show('child', fn() => $s->child());
    show('outside', fn() => add($s, 1));
    var_dump($s->p);
}

echo "-- a property that is a reference\n";
for ($i = 0; $i < 3; $i++) {
    $u = new U;
    $u->p = 1;
    $r = &$u->p;
    var_dump(add($u, 2), $r);
    $r = PHP_INT_MAX;
    var_dump(add($u, 1), $r);
    unset($r);
    $t = new T;
    $r = &$t->p;
    $r = PHP_INT_MAX;
    // (The TypeError's text is #323's.)
    try {
        add($t, 1);
        echo "typed: no error\n";
    } catch (TypeError $e) {
        echo "typed: TypeError\n";
    }
    var_dump($r, sub($t, 1), $r);
    unset($r);
}

echo "-- hooks\n";
class H {
    public int $p = 1 {
        get { echo "get\n"; return $this->p; }
        set { echo "set $value\n"; $this->p = $value; }
    }
    public int $q = 1;
    public function q() { return $this->q += 1; }
}
for ($i = 0; $i < 3; $i++) {
    $h = new H;
    var_dump(add($h, 2), mul($h, 3), $h->q(), $h->q());
}

echo "-- magic, unset and undefined\n";
class M {
    public $p = 1;
    private $hidden = 5;
    private $data = [];
    public function __get($n) { echo "__get $n\n"; return $this->data[$n] ?? 40; }
    public function __set($n, $v) { echo "__set $n\n"; $this->data[$n] = $v; }
}
function addq($o, $y) { return $o->q += $y; }
function addhidden($o, $y) { return $o->hidden += $y; }
for ($i = 0; $i < 3; $i++) {
    $m = new M;
    var_dump(add($m, 1));
    unset($m->p);
    var_dump(add($m, 2), add($m, 2));
    var_dump(addq($m, 1), addq($m, 1));
    var_dump(addhidden($m, 1));
}
class UT { public int $p; }
for ($i = 0; $i < 3; $i++) {
    $t = new UT;
    show('uninitialized', fn() => add($t, 1));
    $t->p = 1;
    show('initialized', fn() => add($t, 1));
    unset($t->p);
    show('unset', fn() => add($t, 1));
    $u = new U;
    unset($u->p);
    show('unset untyped', fn() => add($u, 1));
    show('again', fn() => add($u, PHP_INT_MAX));
    $s = new stdClass;
    $s->p = PHP_INT_MAX;
    show('dynamic', fn() => add($s, 1));
    $s->p = 3;
    show('dynamic', fn() => mul($s, 4));
}

echo "-- \$this and a constant operand\n";
class K {
    public int $n = 0;
    public $u = 0;
    public function run() { $this->n += 2; $this->n *= 3; $this->n -= 1; return $this->n; }
    public function over() { $this->u = PHP_INT_MAX; return $this->u += 1; }
    public function under() { $this->u = PHP_INT_MIN; return $this->u -= 1; }
    public function times() { $this->u = PHP_INT_MAX; return $this->u *= 2; }
    public function typed() { $this->n = PHP_INT_MAX; return $this->n += 1; }
}
for ($i = 0; $i < 3; $i++) {
    $k = new K;
    var_dump($k->run(), $k->run(), $k->over(), $k->under(), $k->times());
    show('typed', fn() => $k->typed());
    var_dump($k->n);
}

echo "-- one name, two properties: the private one of the method's class\n";
class A {
    private $p = 1;
    public function a() { return $this->p += 1; }
}
class B extends A {
    public $p = 100;
    public function b() { return $this->p *= 2; }
}
for ($i = 0; $i < 3; $i++) {
    $b = new B;
    var_dump($b->a(), $b->b(), $b->a(), $b->b(), add($b, 1));
}

echo "-- the value the result replaces is released at the assignment\n";
class D {
    public function __destruct() { echo "destruct\n"; }
}
for ($i = 0; $i < 3; $i++) {
    $u = new U;
    $x = new D;
    echo "before\n";
    $x = ($u->p += 1);
    echo "after\n";
    var_dump($x);
}

echo "-- the other operators\n";
foreach (['U', 'T'] as $c) {
    foreach (['div', 'mod', 'pow_', 'cat', 'shl', 'bor'] as $f) {
        foreach ([2, 0, 64, -1] as $y) {
            $o = new $c;
            $o->p = 7;
            op("$c $f $y", $o, $f, $y);
        }
    }
}

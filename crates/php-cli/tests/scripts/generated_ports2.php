<?php
// The second batch of loop ports (#375) and their cold cases: string
// offsets and quiet reads (FETCH_DIM), `$x::class` (CLASS_NAME), static
// calls that send their variables (StaticCallVars), by-reference sends to
// a method (SendVarMethod), typed variadics and references at function
// entry (VerifyArgs), variadic lists (CollectVariadic), closures
// (MakeClosure), `[...$x]` (ArrayUnpack). Each case runs twice, so the
// second run goes through the caches the first filled.
// No `@`; the expected output is PHP 8.6's.

echo "-- string offsets\n";
$s = 'hello';
for ($r = 0; $r < 2; $r++) {
    $i = 2;
    var_dump($s[0], $s[-1], $s[$i], $s[-5], $s[5] ?? 'none', $s[-6] ?? 'none', empty($s[1]), empty($s[9]), $s['x'] ?? 'q');
    echo $s[7], "|\n";
    echo $s[-9], "|\n";
    $z = '0';
    var_dump(empty($z[0]), isset($z[1]));
}

echo "-- quiet reads of null, scalars, missing keys\n";
$nul = null;
$int = 5;
$f = 1.5;
$b = true;
$a = ['x' => 0, 'y' => 'v', 3 => null];
for ($r = 0; $r < 2; $r++) {
    var_dump($nul['k'] ?? 'd', $int['k'] ?? 'd', $f[0] ?? 'd', $b['k'] ?? 'd', $undef['k'] ?? 'd', $a[3] ?? 'd', $a['q'] ?? 'd');
    var_dump(empty($nul['k']), empty($a['x']), empty($a['y']), empty($a['z']), empty($undef2[1]));
    var_dump($nul['k']);
    var_dump($int[0]);
    $ref = &$a;
    var_dump($ref['y'] ?? 'd', $ref['none'] ?? 'd', empty($ref['x']));
    unset($ref);
}

echo "-- \$x::class\n";
class P {}
class Q extends P {}
$objs = [new P, new Q, new P];
foreach ($objs as $o) {
    echo $o::class, ' ';
}
echo "\n";
$or = new Q;
$rr = &$or;
echo $rr::class, "\n";

echo "-- static calls sending variables\n";
class A {
    public static function inc(array &$a, int $by = 1) { $a[0] += $by; return $a[0]; }
    public static function val(array $a) { return count($a); }
    public static function sum(int ...$xs) { return array_sum($xs); }
    public static function keys(...$xs) { return implode(',', array_keys($xs)); }
    public static function swap(&$x, &$y) { [$x, $y] = [$y, $x]; }
    public function bump(&$x) { $x++; return $x; }
    public function both($v, &$x) { $x .= $v; return $x; }
}
class B extends A {
    public static function inc(array &$a, int $by = 1) { return parent::inc($a, $by * 2); }
}
$arr = [1];
for ($k = 0; $k < 3; $k++) {
    A::inc($arr);
    B::inc($arr, 10);
}
var_dump($arr[0], A::val($arr));
$x = 'x';
$y = 'y';
A::swap($x, $y);
A::swap($x, $y);
A::swap($x, $y);
var_dump($x, $y);
for ($k = 0; $k < 2; $k++) {
    try {
        var_dump(A::val($nope));
    } catch (TypeError $e) {
        echo get_class($e), "\n";
    }
}
$one = 1;
$two = 2;
var_dump(A::sum($one, $two, 3), A::sum(), A::sum($one));
try {
    A::sum($one, 'x');
} catch (TypeError $e) {
    echo get_class($e), "\n";
}
var_dump(A::sum($one, '7'));
echo A::keys($one, $two), ' ', A::keys(a: $one, b: $two), ' ', A::keys($one, b: $two), "\n";
$fl = 2.0;
var_dump(A::sum($fl, $one));

echo "-- method by-reference sends\n";
$obj = new A;
$c = 0;
for ($k = 0; $k < 3; $k++) {
    $obj->bump($c);
}
var_dump($c);
$str = '';
for ($k = 0; $k < 3; $k++) {
    $obj->both($k, $str);
}
var_dump($str);
$obj->bump($fresh);
var_dump($fresh);

echo "-- typed parameters holding references\n";
function first(array &$a) { return $a[0] ?? null; }
function total(int ...$n) { return array_sum($n); }
$list = [4, 5];
for ($k = 0; $k < 2; $k++) {
    var_dump(first($list), total(...$list), total(1, 2, 3));
}
$i5 = 5;
$refd = &$i5;
var_dump(total($refd, $i5));
$sv = '3';
var_dump(total($sv));
$notint = [1];
try {
    total(1, $notint);
} catch (TypeError $e) {
    echo get_class($e), "\n";
}

echo "-- closures\n";
class M {
    public $v = 'm';
    public function make() { return function () { return $this->v; }; }
    public function makeStatic() { return static function () { return isset($this); }; }
    public function arrow() { return fn ($p) => $this->v . $p; }
    public static function st() { return fn () => static::class; }
}
class N extends M { public $v = 'n'; }
foreach ([new M, new N] as $m) {
    $f1 = $m->make();
    $f2 = $m->makeStatic();
    $f3 = $m->arrow();
    $f4 = $m::st();
    var_dump($f1(), $f2(), $f3('!'), $f4());
}
$fs = [];
for ($k = 0; $k < 3; $k++) {
    $fs[] = function () use ($k) { return $k; };
}
echo $fs[0](), $fs[1](), $fs[2](), ' ', $fs[0] === $fs[1] ? 'same' : 'distinct', "\n";

echo "-- unpacking\n";
$l = [1, 2];
$m2 = ['k' => 'v', 5 => 'five'];
for ($k = 0; $k < 2; $k++) {
    var_dump([...$l, ...$m2, ...$l]);
}
$e = [];
var_dump([...$e], [0, ...$e, ...[]]);
$withRef = [1, 2];
$rf = &$withRef[0];
$u = [...$withRef];
$rf = 9;
var_dump($u, $withRef);
function gen() { yield 'a' => 1; yield 2; }
var_dump([...gen()]);
try {
    $bad = 5;
    var_dump([...$bad]);
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
$over = [PHP_INT_MAX => 1];
try {
    var_dump([...$over, ...[1]]);
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}

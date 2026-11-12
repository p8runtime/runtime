<?php
// The ninth batch of loop ports (#376) and their cold cases: what each arm
// does not answer runs out of line on its operands, with step's errors and
// warnings: class constants (first reads, pending constants, the access
// errors), property reads and writes (hooks, __get / __isset / __set,
// dynamic, readonly and typed properties, non-objects), global constants
// (undefined, deprecated), instanceof of a class in a register, static
// properties of the called class, return type coercions, comparisons of
// arrays and `<=>`, `~`, ArrayAccess writes, compound assignments. Each
// case runs twice, so the second run goes through the caches the first
// filled. The expected output is PHP 8.6's.

function show(Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }

spl_autoload_register(function ($c) {
    if ($c === 'Late') {
        eval('class Late { const X = "late-x"; public static $s = "late-s"; }');
    }
});

interface HasK { const K = 'iface'; }
trait T { const TC = 'trait'; public function tc() { return self::TC; } }
enum Suit: string { case Hearts = 'h'; case Spades = 's'; const Wild = self::Spades; }
class Base implements HasK {
    const X = 'base';
    const Sum = Other::Y * 2;
    private const P = 'private';
    protected const R = 'protected';
    public static $sp = 'base-static';
    private static $hidden = 'hidden';
    public static function x() { return static::X; }
    public static function sp() { return static::$sp; }
    public static function setSp($v) { static::$sp = $v; return static::$sp; }
    public function p() { return self::P . ' ' . static::R; }
}
class Kid extends Base { use T; const X = 'kid'; public static $sp = 'kid-static'; }
class Other { const Y = 21; }
class Loop { const A = self::B; const B = self::A; }
class Loop2 { const A = self::A; }
function dflt($v = Base::X . '-default', $w = [Kid::X, Suit::Hearts]) { return $v . ' ' . $w[0] . ' ' . $w[1]->value; }
// A closure bound to no class: `self` and `static` have none to name.
$unscoped = Closure::bind(function () { return self::X; }, null, null);
$unscoped_static = Closure::bind(function () { return static::class; }, null, null);

echo "-- class constants\n";
for ($r = 0; $r < 2; $r++) {
    echo Base::X, ' ', Kid::X, ' ', Base::x(), ' ', Kid::x(), ' ', Base::Sum, ' ', Kid::K, ' ', Late::X, "\n";
    echo dflt(), ' ', dflt('given'), ' ', Suit::Wild->name, ' ', (new Kid)->p(), ' ', (new Kid)->tc(), "\n";
    foreach (['Base::P', 'Base::R', 'Base::NOPE', 'Loop::A', 'Missing::X'] as $c) {
        try { echo constant($c), "\n"; } catch (Throwable $e) { show($e); }
    }
    try { echo Base::P, "\n"; } catch (Throwable $e) { show($e); }
    try { echo Base::R, "\n"; } catch (Throwable $e) { show($e); }
    try { echo T::TC, "\n"; } catch (Throwable $e) { show($e); }
    try { echo Base::NOPE, "\n"; } catch (Throwable $e) { show($e); }
    try { echo Loop2::A, "\n"; } catch (Throwable $e) { show($e); }
    try { echo Missing::X, "\n"; } catch (Throwable $e) { show($e); }
    try { echo $unscoped(), "\n"; } catch (Throwable $e) { show($e); }
    try { echo $unscoped_static(), "\n"; } catch (Throwable $e) { show($e); }
}

class Magic {
    private $data = ['a' => 1];
    public $plain = 'plain';
    public int $typed;
    private $secret = 's';
    public int $hooked { get => 40 + 2; }
    public array $bag = [] { set(array $v) { $this->bag = array_map('strtoupper', $v); } }
    public function __get($n) { echo "__get($n) "; return $this->data[$n] ?? null; }
    public function __isset($n) { echo "__isset($n) "; return isset($this->data[$n]); }
}
class Setter { public function __set($n, $v) { echo "__set($n, ", json_encode($v), ") "; } }
class Plain { public $p = 1; public readonly int $ro; public int $i = 0; public ?Plain $next = null; public function __construct() { $this->ro = 7; } }

echo "-- property reads\n";
for ($r = 0; $r < 2; $r++) {
    $m = new Magic; $nul = null; $int = 5;
    echo $m->a, ' ', var_export($m->b, true), ' ', $m->a ?? 'unset', ' ', $m->b ?? 'dflt', ' ', $m->plain, ' ', $m->hooked, "\n";
    echo var_export($nul?->p, true), ' ', $nul?->p ?? 'null-dflt', ' ', $nul?->p->q ?? 'chain', "\n";
    $o = new Plain; unset($o->p);
    echo var_export($o->p ?? 'gone', true), "\n";
    echo var_export($o->p, true), "\n";
    echo var_export($nul->p, true), ' ', var_export($int->p, true), "\n";
    echo var_export((new Plain)->undeclared, true), "\n";
    try { echo $m->typed, "\n"; } catch (Throwable $e) { show($e); }
    try { echo $m->secret, "\n"; } catch (Throwable $e) { show($e); }
    $m->bag = ['x', 'y']; echo implode(',', $m->bag), "\n";
}

echo "-- property writes\n";
for ($r = 0; $r < 2; $r++) {
    $s = new stdClass; $s->a = 1; $s->{'b'} = [2]; $s->c = 'three';
    echo json_encode($s), "\n";
    $k = new Kid; $k->dyn = 'dynamic'; echo $k->dyn, "\n";
    $set = new Setter; $set->x = [1]; $set->y = 'two'; echo "\n";
    $p = new Plain; $p->i = '12'; $p->next = new Plain; $p->next->i = 3.0;
    echo $p->i, ' ', var_export($p->next->i, true), "\n";
    try { $p->ro = 8; } catch (Throwable $e) { show($e); }
    try { $p->i = 'twelve'; } catch (Throwable $e) { show($e); }
    try { $p->next = 'no'; } catch (Throwable $e) { show($e); }
    $nul = null;
    try { $nul->p = 1; } catch (Throwable $e) { show($e); }
    $str = 'str';
    try { $str->p = 1; } catch (Throwable $e) { show($e); }
    echo $p->i = 99, ' ', $s->z = 'used', "\n";
}

echo "-- global constants\n";
define('LATE_CONST', 'late');
for ($r = 0; $r < 2; $r++) {
    echo PHP_INT_SIZE, ' ', LATE_CONST, ' ', M_PI > 3 ? 'pi' : '?', "\n";
    echo E_STRICT, "\n";
    try { echo NOPE_CONST, "\n"; } catch (Throwable $e) { show($e); }
    if ($r === 0) { define('DEFINED_LATER', 'now'); }
    echo DEFINED_LATER, "\n";
}

echo "-- instanceof a class in a register\n";
for ($r = 0; $r < 2; $r++) {
    $k = new Kid; $names = ['Base', 'Kid', 'HasK', 'Other', 'NoSuchClass', 'kid'];
    foreach ($names as $c) { echo $c, '=', var_export($k instanceof $c, true), ' '; }
    $obj = new Base; $str = 'x';
    echo var_export($k instanceof $obj, true), ' ', var_export($obj instanceof $k, true), ' ', var_export($str instanceof $names[0], true), ' ', var_export(null instanceof $obj, true), "\n";
}

echo "-- static properties\n";
for ($r = 0; $r < 2; $r++) {
    echo Base::sp(), ' ', Kid::sp(), ' ', Late::$s, ' ', Base::setSp("b$r"), ' ', Kid::setSp("k$r"), ' ', Base::$sp, ' ', Kid::$sp, "\n";
    try { echo Base::$nope, "\n"; } catch (Throwable $e) { show($e); }
    try { echo Base::$hidden, "\n"; } catch (Throwable $e) { show($e); }
    try { Base::$hidden = 1; } catch (Throwable $e) { show($e); }
    try { echo Missing::$x, "\n"; } catch (Throwable $e) { show($e); }
}

echo "-- return types\n";
function ret_int($v): int { return $v; }
function ret_str($v): string { return $v; }
function ret_none(): int { if (false) { return 1; } }
function ret_nullable($v): ?float { return $v; }
class Str { function __toString(): string { return 'stringable'; } }
for ($r = 0; $r < 2; $r++) {
    echo var_export(ret_int('5'), true), ' ', var_export(ret_int(7.0), true), ' ', var_export(ret_int(true), true), ' ', var_export(ret_str(12), true), ' ', ret_str(new Str), ' ', var_export(ret_nullable(3), true), ' ', var_export(ret_nullable(null), true), "\n";
    try { ret_int('five'); } catch (Throwable $e) { show($e); }
    try { ret_int(1.5); } catch (Throwable $e) { show($e); }
    try { ret_str([]); } catch (Throwable $e) { show($e); }
    try { ret_none(); } catch (Throwable $e) { show($e); }
}

echo "-- comparisons\n";
for ($r = 0; $r < 2; $r++) {
    $a = [1, 2]; $b = [1, 3]; $c = ['x' => 1]; $i = 3; $s = 'abc'; $o1 = new Plain; $o2 = new Plain;
    echo var_export($a == [1, 2], true), ' ', var_export($a < $b, true), ' ', $a <=> $b, ' ', $b <=> $a, ' ', $i <=> 2, ' ', 2 <=> $i, ' ', 'abd' <=> $s, ' ', $s <=> 'abd', ' ', 1 <=> $a, "\n";
    echo var_export($a == $c, true), ' ', var_export($a < $c, true), ' ', var_export($c > $a, true), ' ', var_export($o1 == $o2, true), ' ', $o1 <=> $o2, ' ', var_export($a === [1, 2], true), ' ', var_export('10' == '1e1', true), ' ', var_export(null < -1, true), "\n";
}

echo "-- bitwise not\n";
for ($r = 0; $r < 2; $r++) {
    $i = 5; $f = 1.5; $s = "ab"; $neg = -1;
    echo ~$i, ' ', ~$neg, ' ', ~$f, ' ', bin2hex(~$s), ' ', ~PHP_INT_MAX, "\n";
    foreach ([[], null, new Plain] as $bad) {
        try { var_dump(~$bad); } catch (Throwable $e) { show($e); }
    }
    $big = 1e30;
    try { var_dump(~$big); } catch (Throwable $e) { show($e); }
}

echo "-- array writes\n";
class Box implements ArrayAccess {
    public $items = [];
    function offsetExists($k): bool { return isset($this->items[$k]); }
    function offsetGet($k): mixed { return $this->items[$k] ?? null; }
    function offsetSet($k, $v): void { echo 'set(', var_export($k, true), ') '; if ($k === null) { $this->items[] = $v; } else { $this->items[$k] = $v; } }
    function offsetUnset($k): void { unset($this->items[$k]); }
}
for ($r = 0; $r < 2; $r++) {
    $box = new Box; $box['a'] = 1; $box[2] = 'two'; $box[] = 'pushed'; echo $box['b'] = 'used', ' ', json_encode($box->items), "\n";
    $str = 'hello'; $str[0] = 'J'; $str[7] = '!'; echo $str, "|\n";
    $arr = []; $arr[1.5] = 'f'; $arr[true] = 't'; $arr[null] = 'n'; echo json_encode($arr), "\n";
    try { $arr[[]] = 1; } catch (Throwable $e) { show($e); }
    $int = 5;
    try { $int[0] = 1; } catch (Throwable $e) { show($e); }
    try { $int[] = 1; } catch (Throwable $e) { show($e); }
    try { $str[] = 'x'; } catch (Throwable $e) { show($e); }
    $nul = null; $nul[] = 'from-null'; echo json_encode($nul), "\n";
}

echo "-- compound assignments\n";
for ($r = 0; $r < 2; $r++) {
    $s = 'a'; $s .= new Str; $o = new Str; $o .= '!'; $arr = [1]; $arr += [5, 6]; $f = 1; $f += 0.5; $n = '3'; $n *= '4';
    echo $s, ' ', $o, ' ', json_encode($arr), ' ', $f, ' ', $n, "\n";
    $x = 'x'; $x .= 5; $x .= 1.5; $x .= true; $x .= null; echo $x, "\n";
    $w = '5 apples'; $w += 1; echo $w, "\n";
    $q = []; try { $q -= 1; } catch (Throwable $e) { show($e); }
    $d = 1; try { $d /= 0; } catch (Throwable $e) { show($e); }
    $m = 1; try { $m %= 0; } catch (Throwable $e) { show($e); }
    $p = new Plain; try { $p .= 'x'; } catch (Throwable $e) { show($e); }
    unset($u); $u += 2; echo $u, "\n";
}

echo "-- unset of properties, variables by name\n";
class Unsetter { public $a = 1; public function __unset($n) { echo "__unset($n) "; } }
function by_name($name) { $local = 'local'; $r = &$$name; $r = "set-$name"; return $local . ' ' . ($r ?? ''); }
function globals_by_name($name, $v) { $GLOBALS[$name] = $v; $g = &$GLOBALS[$name]; $g .= '!'; }
for ($r = 0; $r < 2; $r++) {
    $s = new stdClass; $s->x = 1; $s->y = 2; unset($s->x); $prop = 'y'; unset($s->$prop); unset($s->never); echo json_encode($s), "\n";
    $u = new Unsetter; unset($u->a); unset($u->a); unset($u->b); echo "\n";
    $p = new Plain;
    try { unset($p->ro); } catch (Throwable $e) { show($e); }
    $m = new Magic;
    try { unset($m->hooked); } catch (Throwable $e) { show($e); }
    try { unset($m->secret); echo "unset private from outside\n"; } catch (Throwable $e) { show($e); }
    $nul = null; unset($nul->p); $int = 1; unset($int->p); echo "non-objects\n";
    echo by_name('local'), ' ', by_name('fresh'), "\n";
    globals_by_name('gx', "v$r"); echo $gx, "\n";
    $vn = 'main_var'; $ref = &$$vn; $ref = "main-$r"; echo $main_var, "\n";
}

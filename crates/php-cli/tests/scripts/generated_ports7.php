<?php
// The seventh batch of loop ports (#375) and their cold cases: property
// caches filled again on a class they have not seen, private and
// protected class constants cached per scope, dynamic property names,
// isset of properties and of paths, calls not resolved yet, static calls
// out of line, `new` of a class in a register and of a Throwable,
// references to and from elements and static properties, static property
// op-assignments, generators, includes and declarations, unset. Each case
// runs twice, so the second run goes through the caches the first filled.
// No `@` except where the case is the silenced include; the expected
// output is PHP 8.6's.

class Base {
    public $pub = 'pub';
    protected $prot = 'prot';
    private $priv = 'base-priv';
    private const SECRET = 'secret';
    protected const SHARED = 'shared';
    public const OPEN = 'open';
    function read() { return $this->pub . '/' . $this->prot . '/' . $this->priv . '/' . self::SECRET . '/' . static::SHARED . '/' . self::OPEN; }
    function write($v) { $this->pub = $v; $this->priv = "priv-$v"; }
    function has() { return var_export(isset($this->pub), true) . var_export(isset($this->priv), true); }
}
class Kid1 extends Base { private $priv = 'kid1-priv'; protected const SHARED = 'kid1-shared'; }
class Kid2 extends Base { public $extra = 1; }
class Kid3 extends Base { function own() { return isset($this->priv) ? 'sees' : 'does not see'; } }

echo "-- property caches across classes\n";
for ($r = 0; $r < 2; $r++) {
    foreach ([new Base, new Kid1, new Kid2, new Kid3] as $o) {
        $o->write(get_class($o));
        echo $o->read(), ' ', $o->has(), "\n";
    }
    echo (new Kid3)->own(), "\n";
    try {
        echo (new Kid1)->prot, "\n";
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    try {
        echo Base::SECRET, "\n";
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    $peek = function () { return self::SECRET . ' ' . static::SHARED; };
    $peekKid = function () { return static::SHARED . ' ' . self::OPEN; };
    echo Closure::bind($peek, null, Base::class)(), ' ', Closure::bind($peekKid, null, Kid1::class)(), "\n";
}

echo "-- dynamic property names\n";
class Dyn {
    public $a = 1; public int $n = 0; public ?float $f = null; public readonly int $ro;
    private $hidden = 'h';
    function __construct() { $this->ro = 7; }
    function setAll(array $kv) { foreach ($kv as $k => $v) { $this->$k = $v; } }
    function get($k) { return $this->$k; }
}
class DynMagic {
    private $data = ['x' => 1];
    function __get($k) { echo "__get $k\n"; return $this->data[$k] ?? null; }
    function __set($k, $v) { echo "__set $k\n"; $this->data[$k] = $v; }
    function __isset($k) { echo "__isset $k\n"; return isset($this->data[$k]); }
}
for ($r = 0; $r < 2; $r++) {
    $d = new Dyn;
    $d->setAll(['a' => 'A', 'n' => 5, 'f' => 3]);
    $names = ['a', 'n', 'f', 'hidden'];
    foreach ($names as $k) {
        echo $k, '=', var_export($d->get($k), true), ' ', var_export(isset($d->$k), true), "\n";
    }
    try {
        $k = 'hidden';
        echo $d->$k, "\n";
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    try {
        $k = 'ro';
        $d->$k = 8;
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    try {
        $k = 'n';
        $d->$k = 'not a number';
    } catch (TypeError $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    $k = 'fresh';
    $d->$k = 'new';
    echo $d->fresh, ' ', var_export(isset($d->nope), true), "\n";
    $s = new stdClass;
    $k = 'p';
    $s->$k = 1;
    $s->$k += 1;
    echo $s->$k, ' ', var_export(isset($s->$k), true), "\n";
    $m = new DynMagic;
    $k = 'x';
    echo $m->$k, ' ', var_export(isset($m->$k), true), "\n";
    $m->$k = 2;
    $k = 'y';
    echo var_export(isset($m->$k), true), "\n";
    $num = 0;
    $s->$num = 'zero';
    echo $s->{'0'}, "\n";
}

echo "-- isset of paths\n";
class AAP implements ArrayAccess {
    function offsetExists($o): bool { echo "offsetExists $o\n"; return $o === 'in'; }
    function offsetGet($o): mixed { return ['deep' => 1]; }
    function offsetSet($o, $v): void {}
    function offsetUnset($o): void {}
}
for ($r = 0; $r < 2; $r++) {
    $a = ['x' => ['y' => ['z' => 0, 'n' => null]], 's' => 'str', 'o' => new AAP, 1 => [2 => 'two']];
    $k1 = 'x'; $k2 = 'y';
    foreach ([isset($a['x']['y']['z']), isset($a['x']['y']['n']), isset($a['x']['nope']['z']), isset($a[$k1][$k2]), isset($a['1'][2]), isset($a['s'][0]), isset($a['s'][9]), isset($a['s']['k']), isset($a['o']['in']['deep']), isset($a['o']['out']['deep']), isset($undefined_path['a']['b'])] as $set) {
        echo var_export($set, true), ' ';
    }
    echo "\n";
    $ref = &$a;
    echo var_export(isset($ref['x']['y']['z']), true), "\n";
    unset($ref);
}

echo "-- calls, static calls, new\n";
class Stat {
    static function make(...$a) { return new static(...$a); }
    function __construct(public $v = 'none') {}
    static function __callStatic($m, $a) { return "__callStatic $m"; }
}
class StatKid extends Stat {}
abstract class Abs {}
interface Ifc {}
function gen_two() { yield 1; yield 2; }
for ($r = 0; $r < 2; $r++) {
    echo get_class(StatKid::make()), ' ', Stat::make('x')->v, ' ', Stat::missing(), "\n";
    $dt = DateTime::createFromFormat('Y-m-d', '2026-11-11');
    echo $dt->format('Y'), "\n";
    foreach (['Stat', 'StatKid', 'SplStack', 'Exception'] as $cls) {
        $o = new $cls('arg');
        echo get_class($o), ' ';
    }
    echo "\n";
    foreach (['Abs', 'Ifc', 'NoSuchClassAnywhere'] as $cls) {
        try {
            new $cls;
        } catch (Error $e) {
            echo get_class($e), ': ', $e->getMessage(), "\n";
        }
    }
    $e = new RuntimeException('msg', 3);
    echo get_class($e), ' ', $e->getMessage(), ' ', $e->getCode(), ' ', $e->getLine(), "\n";
    echo implode(',', iterator_to_array(gen_two())), "\n";
    if ($r === 0) {
        function declared_late($x) { return "late $x"; }
    }
    echo declared_late($r), "\n";
    $len = strlen(...);
    $meth = (new Stat('m'))->__construct(...);
    $st = Stat::make(...);
    echo $len('abcd'), ' ', get_class($st()), ' ', get_class($meth), "\n";
}

echo "-- references\n";
class SR { public static $n = 1; private static $p = 'p'; public static $list = []; static function privRef() { $r = &self::$p; $r .= '!'; return self::$p; } }
for ($r = 0; $r < 2; $r++) {
    $a = [];
    $e = &$a['x']['y'];
    $e = 'made';
    $last = &$a['list'][];
    $last = 'appended';
    unset($e, $last);
    echo json_encode($a), "\n";
    $x = 1;
    $a['bound'] = &$x;
    $x = 2;
    $a['deep']['er'] = &$x;
    $x = 3;
    echo $a['bound'], ' ', $a['deep']['er'], "\n";
    $s = &SR::$n;
    $s += 10;
    $l = &SR::$list;
    $l[] = $r;
    echo SR::$n, ' ', json_encode(SR::$list), ' ', SR::privRef(), "\n";
    unset($s, $l);
    try {
        $u = &SR::$undefined;
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    $str = 'abc';
    try {
        $c = &$str[0];
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    $p = 1;
    $q = &$p;
    $q = 5;
    $q = &$q;
    echo $p, "\n";
    SR::$n .= '-cat';
    SR::$n = 1;
    SR::$n += 2;
    SR::$n *= 3;
    echo SR::$n, "\n";
    try {
        SR::$undefined += 1;
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    try {
        SR::$n %= 0;
    } catch (DivisionByZeroError $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    SR::$n = 1;
}

echo "-- generators\n";
function counter_gen($from) {
    $got = yield $from => 'first';
    echo "got $got\n";
    try {
        yield 'k' => 'second';
    } catch (Exception $e) {
        echo "caught ", $e->getMessage(), "\n";
        yield 'after';
    }
    yield from inner_gen();
    return 'ret';
}
function inner_gen() { yield 'inner1'; $x = yield 'inner2'; return $x; }
for ($r = 0; $r < 2; $r++) {
    $g = counter_gen(10);
    echo $g->key(), ' ', $g->current(), "\n";
    echo $g->send('sent'), "\n";
    echo $g->throw(new Exception('boom')), "\n";
    for ($g->next(); $g->valid(); $g->next()) { echo $g->key(), '=', $g->current(), "\n"; }
    echo $g->getReturn(), "\n";
    $lazy = (function () { foreach ([1, 2, 3] as $i) { yield $i * 2; } })();
    echo implode(',', iterator_to_array($lazy, false)), "\n";
    $deleg = (function () { $r = yield from (function () { yield 1; return 'inner-ret'; })(); yield $r; })();
    echo implode(',', iterator_to_array($deleg, false)), "\n";
}

echo "-- includes, declarations, unset\n";
function unset_in_function() { $local = new class { function __destruct() { echo "destructed\n"; } }; unset($local); echo "after unset\n"; }
for ($r = 0; $r < 2; $r++) {
    $v = @include 'no-such-file-for-the-script.php';
    var_dump($v);
    unset_in_function();
    $gone = 1;
    unset($gone);
    var_dump(isset($gone));
    if ($r === 0) {
        class DeclaredInLoop { const C = 'c'; }
    }
    echo DeclaredInLoop::C, "\n";
}
echo "-- scalars, dynamic properties, cases, references, casts, variable variables\n";
enum Suit: string { case Hearts = 'h'; case Spades = 's'; const Wild = self::Spades; }
$GLOBALS['glob_for_test'] = 'g';
function read_globals() { return $GLOBALS['glob_for_test'] . ' ' . (isset($_SERVER['argv']) ? 'argv' : 'no argv'); }
for ($r = 0; $r < 2; $r++) {
    $nul = null; $str = 'abc'; $int = 5; $bool = true;
    foreach ([isset($nul['k']), isset($str[0]), isset($str[-1]), isset($str[-4]), isset($str[3]), isset($int[0]), isset($bool['x']), isset($str['1']), isset($str['x'])] as $set) {
        echo var_export($set, true), ' ';
    }
    echo "\n";
    $s = new stdClass;
    $s->a = 1;
    $s->n = null;
    $k = 'a';
    echo $s->a, ' ', $s->$k, ' ', var_export(isset($s->a), true), var_export(isset($s->n), true), var_export(isset($s->none), true), "\n";
    echo $s->none ?? 'quiet', "\n";
    echo Suit::Hearts->value, ' ', Suit::Wild->name, ' ', Suit::from('s')->name, ' ', var_export(Suit::Hearts === Suit::Hearts, true), "\n";
    $arr = [1, 'k' => 2, 'p' => [3]];
    $r1 = &$arr[0]; $r2 = &$arr['k']; $r3 = &$arr['p'][0];
    $arr[0] = 'one'; $arr['k'] = 'two'; $arr['p'][0] = 'three';
    echo $r1, ' ', $r2, ' ', $r3, "\n";
    unset($r1, $r2, $r3);
    $o = (object) ['x' => 1, 'y' => [2]];
    echo json_encode($o), ' ', get_class((object) null), ' ', json_encode((object) 'str'), ' ', var_export((object) $o === $o, true), "\n";
    $vn = 'str';
    echo $$vn, "\n";
    $missing = 'nope_' . $r;
    echo var_export($$missing, true), "\n";
    echo read_globals(), "\n";
    $x = 1;
    $lit = [&$x, 2];
    $lit[0] = 10;
    echo $x, "\n";
}
echo "done\n";

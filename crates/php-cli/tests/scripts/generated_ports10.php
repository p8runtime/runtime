<?php
// The tenth batch of loop ports (#376) and their cold cases: what each arm
// does not answer runs out of line on its operands, with step's errors and
// warnings: the intrinsics' first calls and their errors, binary operators
// (Stringables, array unions both ways, the operand errors, division by
// zero), dynamic property names (writes and reads, magic, readonly, typed,
// non-objects), by-reference questions of functions and static methods,
// static sends, echo and print, clone, throw and catch, isset() of static
// properties, ArrayAccess reads and isset(), empty() and string offsets.
// Each case runs twice, so the second run goes through the caches the
// first filled. The expected output is PHP 8.6's.

function show(Throwable $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }

spl_autoload_register(function ($c) {
    if ($c === 'LateS') {
        eval('class LateS { public static $s = "late"; static function m(&$x) { $x = "by-ref"; return $x; } }');
    }
});

class Str { public function __construct(public string $s = 's') {} function __toString(): string { return $this->s; } }
class NoStr {}
class Magic {
    private $d = ['a' => 1];
    function __get($n) { echo "get $n\n"; return $this->d[$n] ?? null; }
    function __isset($n) { echo "isset $n\n"; return isset($this->d[$n]); }
    function __set($n, $v) { echo "set $n\n"; $this->d[$n] = $v; }
}
class Typed { public int $i = 0; public readonly string $ro; function __construct() { $this->ro = 'r'; } }
class Plain { public $p = 1; }
class Counted implements Countable { function count(): int { return 7; } }
class Box implements ArrayAccess {
    function offsetExists($k): bool { echo 'exists ', var_export($k, true), "\n"; return $k !== 'no'; }
    function offsetGet($k): mixed { echo 'get ', var_export($k, true), "\n"; return $k === 'zero' ? 0 : 5; }
    function offsetSet($k, $v): void {}
    function offsetUnset($k): void {}
}
class Cl { public $n = 0; public array $log = []; function __clone() { $this->n++; } }
class RoCl { public function __construct(public readonly int $v = 1) {} function __clone() { $this->v = 2; } }
class PrivCl { private function __clone() {} }
enum Suit { case Hearts; }
class St {
    public static $s = 1; public static $nul = null; private static $hidden = 2;
    static function go($v) { return static::m($v) . ' ' . static::r($v); }
    static function m($x) { return "m:$x"; }
    static function r(&$x) { $x .= '!'; return "r:$x"; }
    static function arg() { $a = ['k' => 'v']; return static::r($a['k']) . ' ' . $a['k']; }
    static function has() { return var_export([isset(static::$s), isset(static::$nul), isset(static::$hidden), isset(static::$none)], true); }
}
class St2 extends St { static function m($x) { return "m2:$x"; } }
class E1 extends Exception {}
class E2 extends Exception {}
interface Tagged {}
class E3 extends Exception implements Tagged {}

echo "-- intrinsics\n";
function intrinsics($s, $a, $o) {
    $r = &$s;
    echo strlen($s), ' ', strlen($r), ' ', count($a), ' ', count($o), ' ', var_export([is_null($s), is_bool($s), is_int($s), is_float($s), is_string($r), is_array($a), is_object($o)], true), ' ', var_export(array_key_exists('x', $a), true), "\n";
    echo strlen(12345), ' ', var_export(array_key_exists(1, $a), true), "\n";
    try { echo strlen([]), "\n"; } catch (Throwable $e) { show($e); }
    try { echo count(5), "\n"; } catch (Throwable $e) { show($e); }
    try { echo count(new Plain), "\n"; } catch (Throwable $e) { show($e); }
    try { echo array_key_exists('x', 5), "\n"; } catch (Throwable $e) { show($e); }
}
for ($r = 0; $r < 2; $r++) {
    intrinsics('hello', ['x' => 1, 1 => 2], new Counted);
}

echo "-- binary\n";
for ($r = 0; $r < 2; $r++) {
    $str = new Str('S'); $arr = [1 => 'a']; $num = '6'; $f = 7.5; $z = 0; $neg = -1;
    echo $str . $str, ' ', 'p' . $str, ' ', $str . 'q', ' ', $str . 1, "\n";
    echo json_encode([5, 6] + $arr), ' ', json_encode($arr + [5, 6]), ' ', $num - '2', ' ', $num * '2', ' ', $num + '1.5', ' ', 10 - $num, "\n";
    echo $f % 2, ' ', 2 ** $num, ' ', $num / 4, ' ', $num | '1', "\n";
    echo @('x' . $arr), "\n";
    echo 'a' . [1], "\n";
    try { echo 'x' . new NoStr, "\n"; } catch (Throwable $e) { show($e); }
    try { echo new NoStr . 'x', "\n"; } catch (Throwable $e) { show($e); }
    try { echo $arr + 1, "\n"; } catch (Throwable $e) { show($e); }
    try { echo 1 + $arr, "\n"; } catch (Throwable $e) { show($e); }
    try { echo [1] - $num, "\n"; } catch (Throwable $e) { show($e); }
    try { echo $num * [], "\n"; } catch (Throwable $e) { show($e); }
    try { echo $num / $z, "\n"; } catch (Throwable $e) { show($e); }
    try { echo $num % $z, "\n"; } catch (Throwable $e) { show($e); }
    try { echo 1 << $neg, "\n"; } catch (Throwable $e) { show($e); }
    try { echo 'abc' * $num, "\n"; } catch (Throwable $e) { show($e); }
    echo '5 apples' + $num, "\n";
    echo "i:{$str}:{$num}:{$f}", "\n";
    echo "a:{$arr}", "\n";
    $nos = new NoStr;
    try { echo "x{$nos}y", "\n"; } catch (Throwable $e) { show($e); }
}

echo "-- dynamic property names\n";
for ($r = 0; $r < 2; $r++) {
    $n = 'a'; $p = 'p'; $i = 'i'; $ro = 'ro'; $none = 'none';
    $o = new stdClass; $o->$n = 1; $o->$p = $o->$n + 1; echo json_encode($o), "\n";
    $m = new Magic; $m->$p = 3; echo $m->$n, ' ', $m->$p, ' ', $m->$none ?? 'dflt', "\n";
    $pl = new Plain; echo $pl->$p, ' ', $pl->$none ?? 'quiet', "\n";
    echo $pl->$none, "\n";
    $pl->$n = 2; echo $pl->a, "\n";
    $t = new Typed; $t->$i = '5'; var_dump($t->$i);
    try { $t->$i = 'x'; } catch (Throwable $e) { show($e); }
    try { $t->$ro = 'w'; } catch (Throwable $e) { show($e); }
    $nul = null; $int = 3;
    echo $nul->$p ?? 'null-quiet', "\n";
    echo $nul->$p, "\n";
    echo $int->$p, "\n";
    try { $nul->$p = 1; } catch (Throwable $e) { show($e); }
    try { $int->$p = 1; } catch (Throwable $e) { show($e); }
}

echo "-- by-reference questions\n";
if (true) { function late_ref(&$x) { $x = 'set'; } function late_val($x) { return $x; } }
for ($r = 0; $r < 2; $r++) {
    $a = ['k' => 0];
    late_ref($a['k']); echo $a['k'], ' ', late_val($a['k']), "\n";
    try { nope_fn($a['k']); } catch (Throwable $e) { show($e); }
    echo St::go('v'), ' ', St2::go('w'), ' ', St::arg(), ' ', St2::arg(), "\n";
    $v = 'x';
    echo LateS::m($v), ' ', $v, "\n";
    try { echo Missing::m($v), "\n"; } catch (Throwable $e) { show($e); }
    echo St::m($undefined), "\n";
}

echo "-- echo\n";
for ($r = 0; $r < 2; $r++) {
    echo new Str('obj'), ' ', 1.5, ' ', null, ' ', true, ' ', false, ' ', -0.0, "\n";
    print new Str('printed');
    print "\n";
    echo [1], "\n";
    try { echo new NoStr; } catch (Throwable $e) { show($e); }
    ?>inline <?= new Str('short') ?> html
<?php
}

echo "-- clone\n";
for ($r = 0; $r < 2; $r++) {
    $c = new Cl; $c->log[] = 1; $d = clone $c; $d->log[] = 2;
    echo $c->n, ' ', $d->n, ' ', count($c->log), ' ', count($d->log), "\n";
    $ro = new RoCl; $ro2 = clone $ro; echo $ro->v, ' ', $ro2->v, "\n";
    $s = 'str';
    try { $x = clone $s; } catch (Throwable $e) { show($e); }
    try { $x = clone new PrivCl; } catch (Throwable $e) { show($e); }
    try { $x = clone Suit::Hearts; } catch (Throwable $e) { show($e); }
}

echo "-- throw and catch\n";
function thrower($what) {
    try {
        throw $what;
    } catch (E1 $e) {
        return 'E1 ' . $e->getMessage();
    } catch (Tagged | E2 $e) {
        return 'Tagged|E2 ' . get_class($e);
    } catch (NotAClass $e) {
        return 'never';
    }
}
for ($r = 0; $r < 2; $r++) {
    echo thrower(new E1('one')), ' ', thrower(new E2), ' ', thrower(new E3), "\n";
    try { thrower(new Exception('plain')); } catch (Exception $e) { show($e); }
    try { thrower('str'); } catch (Throwable $e) { show($e); }
    try { $v = null ?? throw new E1('expr'); } catch (E1 $e) { show($e); }
}

echo "-- isset of static properties\n";
for ($r = 0; $r < 2; $r++) {
    echo St::has(), ' ', var_export([isset(St::$s), isset(St::$hidden), isset(LateS::$s), isset(St2::$s)], true), "\n";
    try { echo isset(Missing::$s), "\n"; } catch (Throwable $e) { show($e); }
}

echo "-- ArrayAccess reads and isset, empty, offsets\n";
for ($r = 0; $r < 2; $r++) {
    $box = new Box; $k = 'key'; $flags = [1 => 0, '' => 'e']; $s = 'abc'; $int = 4; $nul = null;
    echo $box['5'], ' ', $box[5], ' ', $box[$k], ' ', $box['no'] ?? 'dflt', "\n";
    echo var_export([isset($box['5']), isset($box['no']), isset($box[$k]), empty($box['zero']), empty($box[$k])], true), "\n";
    echo var_export([empty($flags[true]), empty($flags[null]), empty($flags[false]), isset($flags[true]), isset($s[1]), isset($s['1']), isset($s['x']), isset($s[1.0])], true), "\n";
    echo var_export([empty($s[1]), empty($s['x']), empty($s[5]), empty($nul['k']), empty($int[0])], true), "\n";
    echo $s[10] ?? 'oob', ' ', $int[0] ?? 'int', "\n";
    echo $s[10], "\n";
    echo $int[0], "\n";
    echo $nul['k'], "\n";
    try { echo $s['x'], "\n"; } catch (Throwable $e) { show($e); }
    try { echo $flags[[]], "\n"; } catch (Throwable $e) { show($e); }
}

echo "-- foreach of what is not an array in a register\n";
for ($r = 0; $r < 2; $r++) {
    $arr = [1, 2]; $ref = &$arr; $empty = []; $eref = &$empty; $o = new Plain; $oref = &$o; $int = 5; $nul = null;
    foreach ($ref as $v) { echo $v; }
    foreach ($eref as $v) { echo 'never'; }
    foreach ($oref as $k => $v) { echo " $k=$v"; }
    echo "\n";
    foreach ($int as $v) { echo 'never'; }
    foreach ($nul as $v) { echo 'never'; }
    unset($ref, $eref, $oref);
}

echo "-- steps of what is not an int\n";
for ($r = 0; $r < 2; $r++) {
    $f = 1.5; $nul = null; $s = 'a'; $z = 'Az'; $num = '5'; $b = true; $max = PHP_INT_MAX; $min = PHP_INT_MIN; $arr = [];
    $f++; $nul++; $s++; $z++; $num++; $b++; $max++; $min--;
    var_dump($f, $nul, $s, $z, $num, $b, $max, $min);
    $f--; $num--;
    var_dump($f, $num);
    $nul2 = null; $nul2--; var_dump($nul2);
    $empty = ''; $empty--; var_dump($empty);
    try { $arr++; } catch (Throwable $e) { show($e); }
    try { $arr--; } catch (Throwable $e) { show($e); }
    $o = new Plain;
    try { $o++; } catch (Throwable $e) { show($e); }
    $u++; var_dump($u);
    unset($u);
}

echo "-- arguments of their types\n";
function ints(int $a, ?float $b = null, string ...$rest) { return var_export([$a, $b, $rest], true); }
function strict_obj(Plain $p) { return 'plain'; }
for ($r = 0; $r < 2; $r++) {
    echo ints('5', 3, 7, 1.5), "\n";
    echo ints(true), "\n";
    try { echo ints('x'), "\n"; } catch (Throwable $e) { show($e); }
    try { echo ints(1, 'y'), "\n"; } catch (Throwable $e) { show($e); }
    try { echo ints(1, 2, []), "\n"; } catch (Throwable $e) { show($e); }
    try { echo strict_obj(new stdClass), "\n"; } catch (Throwable $e) { show($e); }
    echo ints(1.0), "\n";
    try { echo ints(1.5), "\n"; } catch (Throwable $e) { show($e); }
}

echo "-- compound assignments of elements\n";
for ($r = 0; $r < 2; $r++) {
    $a = ['s' => 'x', 'n' => 1, 'arr' => [1]]; $k = 's'; $box = new Box; $nul = null;
    $a[$k] .= 'y'; $a['n'] .= 2; $a['arr'] += [5, 6]; $a['f'] ??= 'set';
    echo json_encode($a), "\n";
    $a['missing'] .= 'z'; echo $a['missing'], "\n";
    $nul['k'] .= 'n'; echo json_encode($nul), "\n";
    try { $a['arr'] .= 'x'; echo $a['arr'], "\n"; } catch (Throwable $e) { show($e); }
    try { $a['n'] /= 0; } catch (Throwable $e) { show($e); }
    try { $a['s'] -= 1; } catch (Throwable $e) { show($e); }
    $box[$k] .= 'w';
    $str = 'abc';
    try { $str[0] .= 'x'; } catch (Throwable $e) { show($e); }
}

echo "-- steps of elements\n";
for ($r = 0; $r < 2; $r++) {
    $a = ['f' => 1.5, 'n' => null, 's' => 'a', 'num' => '9', 'arr' => [], 'deep' => ['x' => 1.0]]; $k = 'f'; $box = new Box; $str = 'abc'; $i = 0;
    $a[$k]++; $k = 'n'; $a[$k]++; $k = 's'; $a[$k]++; $k = 'num'; $a[$k]--;
    $k = 'deep'; $x = 'x'; $a[$k][$x]++;
    echo json_encode($a), "\n";
    $k = 'missing'; $a[$k]++; echo $a['missing'], "\n";
    $k = 'arr';
    try { $a[$k]++; } catch (Throwable $e) { show($e); }
    $k = 'q'; echo $box[$k]++, "\n";
    try { $str[$i]++; } catch (Throwable $e) { show($e); }
    $nul = null; $k = 'z'; $nul[$k]--; var_dump($nul);
}

echo "-- variadics with named arguments\n";
function vari(...$rest) { return json_encode($rest); }
function vari2($a, ...$rest) { return $a . ' ' . json_encode($rest); }
for ($r = 0; $r < 2; $r++) {
    echo vari(1, 2, x: 3, y: 4), ' ', vari(x: 1), ' ', vari2(1, 2, z: 3), ' ', vari2(a: 5, b: 6), "\n";
    try { echo vari2(1, a: 2), "\n"; } catch (Throwable $e) { show($e); }
}

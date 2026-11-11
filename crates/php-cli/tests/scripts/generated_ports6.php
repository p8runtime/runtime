<?php
// The sixth batch of loop ports (#375) and their cold cases: method calls
// out of line from the MethodCall arm (native methods, a site seeing many
// classes, __call, the errors), sends and by-reference tests on a miss,
// foreach over objects, native constructors, calls of values (closures,
// callables), list() destructuring (FetchList), foreach by reference
// (IterNextRef), a function argument's by-reference test (FuncArgByRef),
// static variables (StaticVar). Each case runs twice, so the second run
// goes through the caches the first filled. No `@`; the expected output
// is PHP 8.6's.

class Noisy {
    function __construct(public $name) {}
    function __destruct() { echo "destruct {$this->name}\n"; }
    function hi() { return "hi {$this->name}"; }
}

class K1 { function v(&$x) { $x .= '1'; return 'K1'; } static function st() { return 'st K1'; } }
class K2 { function v($x) { return 'K2'; } }
class K3 { function v(&$x) { $x .= '3'; return 'K3'; } }
class K4 { function __call($m, $a) { return "__call $m(" . count($a) . ")"; } }
class Vis {
    private function priv() { return 'priv'; }
    protected function prot() { return 'prot'; }
    function inside() { return $this->priv() . '+' . $this->prot(); }
    function gen() { yield 1; yield 2; }
    function need($a, $b) { return $a + $b; }
}

echo "-- method calls\n";
function call_v($o, &$s) { return $o->v($s); }
for ($r = 0; $r < 2; $r++) {
    $s = '';
    foreach ([new K1, new K2, new K3, new K4, new K1] as $o) {
        echo call_v($o, $s), ' ';
    }
    echo $s, "\n";
    $it = new ArrayIterator(['a' => 1, 'b' => 2]);
    echo $it->count(), ' ', $it->key(), ' ', $it->current(), ' ';
    $it->next();
    echo $it->key(), "\n";
    $rc = new ReflectionClass('Vis');
    echo $rc->getName(), ' ', var_export($rc->hasMethod('gen'), true), ' ', $rc->getMethod('need')->getNumberOfParameters(), "\n";
    $k = new K1;
    echo $k->st(), ' ', (new Vis)->inside(), ' ', implode(',', iterator_to_array((new Vis)->gen())), "\n";
    echo (new Noisy('tmp'))->hi(), "\n";
    $held = new K2;
    $ref = &$held;
    echo $ref->v(1), "\n";
    foreach (['priv', 'prot', 'nope'] as $m) {
        try {
            echo (new Vis)->$m(), "\n";
        } catch (Error $e) {
            echo get_class($e), ': ', $e->getMessage(), "\n";
        }
    }
    foreach ([null, 5, 'str'] as $bad) {
        try {
            $bad->v(1);
        } catch (Error $e) {
            echo get_class($e), ': ', $e->getMessage(), "\n";
        }
    }
    try {
        (new Vis)->need(1);
    } catch (ArgumentCountError $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}

echo "-- sends on a miss\n";
function send_all($objs) {
    $out = [];
    foreach ($objs as $o) {
        $a = ['k' => 'x'];
        $o->v($a['k']);
        $b = 'y';
        $o->v($b);
        $out[] = $a['k'] . '/' . $b;
    }
    return implode(' ', $out);
}
for ($r = 0; $r < 2; $r++) {
    echo send_all([new K1, new K2, new K3, new K2, new K4]), "\n";
}

echo "-- foreach over objects\n";
class It implements Iterator {
    private $i = 0;
    function __construct(private array $a, private $fail = -1) {}
    function rewind(): void { echo "rewind\n"; $this->i = 0; }
    function valid(): bool {
        if ($this->i === $this->fail) { throw new RuntimeException("valid at {$this->i}"); }
        return $this->i < count($this->a);
    }
    function current(): mixed { return $this->a[$this->i]; }
    function key(): mixed { return 'k' . $this->i; }
    function next(): void { $this->i++; }
}
class Agg implements IteratorAggregate { function getIterator(): Iterator { return new It(['x', 'y']); } }
class BadAgg implements IteratorAggregate { #[\ReturnTypeWillChange] function getIterator() { return [1]; } }
class Props {
    public $a = 1; protected $b = 2; private $c = 3; public $unset;
    function own() { $out = []; foreach ($this as $k => $v) { $out[] = "$k=$v"; } return implode(',', $out); }
}
function gen_pairs() { yield 'p' => 1; yield 'q' => 2; return 3; }
for ($r = 0; $r < 2; $r++) {
    foreach (new It([10, 20]) as $k => $v) { echo "$k=$v\n"; }
    foreach (new It([]) as $v) { echo "never\n"; }
    foreach (new Agg as $v) { echo $v, "\n"; }
    foreach (new It([1, 2, 3]) as $v) { if ($v === 2) { break; } echo $v, "\n"; }
    try {
        foreach (new It([1, 2, 3], 1) as $v) { echo $v, "\n"; }
    } catch (RuntimeException $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    try {
        foreach (new BadAgg as $v) { echo $v, "\n"; }
    } catch (Exception $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    $g = gen_pairs();
    foreach ($g as $k => $v) { echo "$k=$v\n"; }
    echo $g->getReturn(), "\n";
    $p = new Props;
    unset($p->unset);
    $p->dyn = 4;
    foreach ($p as $k => $v) { echo "$k=$v "; }
    echo "| ", $p->own(), "\n";
    foreach (new ArrayObject(['m' => 1, 'n' => 2]) as $k => $v) { echo "$k=$v "; }
    echo "\n";
}

echo "-- native constructors\n";
class NoCtor {}
class NeedsTwo { function __construct($a, $b) { echo "NeedsTwo $a $b\n"; } }
for ($r = 0; $r < 2; $r++) {
    $ao = new ArrayObject([3, 1, 2]);
    echo count($ao), ' ', $ao[0], "\n";
    $ai = new ArrayIterator(['z' => 26]);
    echo $ai->key(), "\n";
    $rm = new ReflectionMethod('Vis', 'need');
    echo $rm->getName(), ' ', $rm->class, "\n";
    try {
        new ReflectionClass('NoSuchClass');
    } catch (ReflectionException $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    try {
        new NeedsTwo(1);
    } catch (ArgumentCountError $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    new NeedsTwo(1, 2);
    $o = new NoCtor(new Noisy('arg'));
    echo get_class($o), "\n";
}

echo "-- calls of values\n";
class Inv { function __invoke($x) { return "invoked $x"; } function m($x) { return "m $x"; } static function s($x) { return "s $x"; } }
function takes_ref(&$x) { $x .= '!'; return $x; }
for ($r = 0; $r < 2; $r++) {
    $k = 10;
    $add = function ($x) use ($k) { return $x + $k; };
    $acc = 0;
    $into = function ($x) use (&$acc) { $acc += $x; return $acc; };
    $arrow = fn ($x) => $x * $k;
    $static = static fn () => isset($this) ? 'this' : 'no this';
    $bound = Closure::bind(function () { return $this->name; }, new Noisy('bound'), Noisy::class);
    echo $add(1), ' ', $into(2), ' ', $into(3), ' ', $acc, ' ', $arrow(3), ' ', $static(), ' ', $bound(), "\n";
    unset($bound);
    $calls = ['strtoupper', [new Inv, 'm'], 'Inv::s', [Inv::class, 's'], new Inv, 'takes_ref'];
    foreach ($calls as $f) {
        $v = 'v';
        echo $f($v), ' ';
    }
    echo "\n";
    $byref = function (&$x) { $x .= '?'; };
    $v = 'w';
    $byref($v);
    echo $v, "\n";
    $gen = function () { yield 1; yield 2; };
    echo implode(',', iterator_to_array($gen())), "\n";
    try {
        $add();
    } catch (ArgumentCountError $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    foreach (['no_such_function', 5, [new Inv, 'nope']] as $f) {
        try {
            $f(1);
        } catch (Error $e) {
            echo get_class($e), ': ', $e->getMessage(), "\n";
        }
    }
    try {
        $undefined(1);
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}

echo "-- list()\n";
class AA implements ArrayAccess {
    function offsetExists($o): bool { return true; }
    function offsetGet($o): mixed { return "aa[$o]"; }
    function offsetSet($o, $v): void {}
    function offsetUnset($o): void {}
}
for ($r = 0; $r < 2; $r++) {
    [$a, $b, $c] = [1, 2, 3];
    ['x' => $x, 'y' => $y] = ['y' => 'Y', 'x' => 'X'];
    [, $second] = ['skip', 'kept'];
    [[$n1, $n2], $n3] = [[4, 5], 6];
    ['1' => $num] = [1 => 'one'];
    echo "$a $b $c $x $y $second $n1 $n2 $n3 $num\n";
    [$m1, $m2] = [7];
    var_dump($m2);
    [$z1, $z2] = null;
    var_dump($z1, $z2);
    [$s1] = 'str';
    var_dump($s1);
    [$p, $q] = new AA;
    echo "$p $q\n";
    $src = [8, 9];
    $ref = &$src;
    [$r1, $r2] = $ref;
    echo "$r1 $r2\n";
    $inner = 10;
    $withref = [&$inner, 11];
    [$w1, $w2] = $withref;
    echo "$w1 $w2\n";
    foreach ([[1, 'a'], [2, 'b']] as [$id, $name]) { echo "$id:$name "; }
    echo "\n";
}

echo "-- foreach by reference\n";
class Box { public $items = [1, 2, 3]; public $x = 1; public $y = 2; }
for ($r = 0; $r < 2; $r++) {
    $arr = ['a' => 1, 'b' => 2, 'c' => 3];
    foreach ($arr as $k => &$v) { $v = "$k$v"; }
    unset($v);
    echo implode(',', $arr), "\n";
    $list = [1, 2, 3];
    foreach ($list as &$v) { $v *= 10; if ($v === 20) { $list[] = 4; } }
    unset($v);
    echo implode(',', $list), "\n";
    $box = new Box;
    foreach ($box->items as &$v) { $v++; }
    unset($v);
    echo implode(',', $box->items), "\n";
    foreach ($box as $k => &$v) { if (is_int($v)) { $v += 100; } }
    unset($v);
    echo $box->x, ' ', $box->y, "\n";
    $nested = [[1], [2]];
    foreach ($nested as &$inner) { foreach ($inner as &$leaf) { $leaf = -$leaf; } unset($leaf); }
    unset($inner);
    echo json_encode($nested), "\n";
    $empty = [];
    foreach ($empty as &$v) { echo "never\n"; }
    $keep = ['x' => 1];
    $alias = 'old';
    $k2 = &$alias;
    foreach ($keep as $k2 => &$v) { $v = 2; }
    unset($v);
    echo $alias, ' ', $keep['x'], "\n";
}

echo "-- by-reference arguments of functions\n";
function add_one(&$x) { $x = ($x ?? 0) + 1; }
function show($x) { return var_export($x, true); }
for ($r = 0; $r < 2; $r++) {
    $a = ['k' => 1];
    add_one($a['k']);
    add_one($a['new']);
    add_one($a['deep']['er']);
    echo json_encode($a), "\n";
    echo show($a['k']), "\n";
    echo show($a['missing'] ?? 'default'), "\n";
    $b = ['list' => [3, 1, 2]];
    sort($b['list']);
    echo implode(',', $b['list']), ' ', end($b['list']), "\n";
    $o = new stdClass;
    $o->arr = [5];
    array_push($o->arr, 6);
    echo implode(',', $o->arr), "\n";
}
echo show($a['missing']), "\n";

echo "-- static variables\n";
function count_calls() { static $n = 0; static $label = 'calls' . PHP_EOL; return $label . ++$n; }
function fact($n) { static $depth = 0; $depth++; $r = $n <= 1 ? 1 : $n * fact($n - 1); return $r; }
class St {
    function bump() { static $s = 0; return ++$s; }
}
class StChild extends St {}
for ($r = 0; $r < 2; $r++) {
    echo count_calls(), "\n";
    echo fact(5), "\n";
    echo (new St)->bump(), ' ', (new St)->bump(), ' ', (new StChild)->bump(), "\n";
    $cl = function () { static $c = 0; return ++$c; };
    echo $cl(), $cl(), "\n";
}
echo "done\n";

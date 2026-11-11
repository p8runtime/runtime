<?php
// The fourth batch of loop ports (#375) and their cold cases: unset of a
// dimension path (UnsetDim), ++/-- on a dimension path (DimIncDec) and on
// a property (PropIncDec), the scalar casts (Cast), a method argument's
// by-reference test (MethodArgByRef), foreach over objects. Each case runs
// twice, so the second run goes through the caches the first filled.
// No `@`; the expected output is PHP 8.6's.

class Bag implements ArrayAccess {
    public $data = [];
    function offsetExists($k): bool { echo "exists($k)\n"; return isset($this->data[$k]); }
    function offsetGet($k): mixed { echo "get($k)\n"; return $this->data[$k] ?? null; }
    function offsetSet($k, $v): void { echo "set($k)\n"; $this->data[$k] = $v; }
    function offsetUnset($k): void { echo "unset($k)\n"; unset($this->data[$k]); }
}

class Noisy {
    function __construct(public $name) {}
    function __destruct() { echo "destruct {$this->name}\n"; }
}

echo "-- unset of dimensions\n";
for ($r = 0; $r < 2; $r++) {
    $a = ['x' => 1, 'y' => ['z' => 2, 'w' => [3, 4]], 5 => 'five', 'n' => null];
    unset($a['x']);
    unset($a['y']['z']);
    unset($a['y']['w'][0]);
    unset($a['missing']);
    unset($a['missing']['deeper']);
    unset($a['y']['missing']['deeper']);
    unset($a['5']);
    unset($a['n']['k']);
    var_dump($a);
    $k = 'y'; $j = 'w'; $i = 1;
    unset($a[$k][$j][$i]);
    var_dump($a);
    // A copy is not touched (copy-on-write).
    $b = ['p' => ['q' => 1, 'r' => 2]];
    $c = $b;
    unset($c['p']['q']);
    var_dump($b, $c);
    // Keys of other types: float, bool, null.
    $d = [0 => 'zero', 1 => 'one', '' => 'empty', 2 => 'two'];
    unset($d[1.5]);
    unset($d[true]);
    unset($d[null]);
    var_dump($d);
    // Through a reference, and a reference on the way.
    $e = ['a' => ['b' => 1, 'c' => 2]];
    $ref = &$e;
    unset($ref['a']['b']);
    $inner = &$e['a'];
    unset($e['a']['c']);
    var_dump($e, $inner);
    unset($ref, $inner);
    // The value freed by the unset: its destructor runs there.
    $f = ['o' => new Noisy("o$r"), 'p' => ['q' => new Noisy("q$r")]];
    unset($f['o']);
    echo "after o\n";
    unset($f['p']['q']);
    echo "after q\n";
    // Not arrays.
    $nul = null;
    unset($nul['k']);
    unset($nul['k']['j']);
    var_dump($nul);
    unset($undefined_var['k']);
    $str = 'abc';
    try {
        unset($str[0]);
    } catch (Error $e2) {
        echo get_class($e2), ': ', $e2->getMessage(), "\n";
    }
    $arr_str = ['s' => 'abc'];
    try {
        unset($arr_str['s'][0]);
    } catch (Error $e2) {
        echo get_class($e2), ': ', $e2->getMessage(), "\n";
    }
    $int = 5;
    try {
        unset($int['k']);
    } catch (Error $e2) {
        echo get_class($e2), ': ', $e2->getMessage(), "\n";
    }
    $false = false;
    try {
        unset($false['k']);
    } catch (Error $e2) {
        echo get_class($e2), ': ', $e2->getMessage(), "\n";
    }
    var_dump($int, $false);
    $bag = new Bag;
    $bag->data = ['k' => 1, 'j' => 2];
    unset($bag['k']);
    $nested = ['bag' => $bag];
    unset($nested['bag']['j']);
    var_dump($bag->data);
    $obj = ['o' => new stdClass];
    try {
        unset($obj['o']['k']);
    } catch (Error $e2) {
        echo get_class($e2), ': ', $e2->getMessage(), "\n";
    }
    // During a foreach over the array (by value: the loop's copy goes on).
    $g = [1, 2, 3, 4];
    foreach ($g as $gk => $gv) {
        unset($g[$gk + 1]);
        echo "$gk=$gv ";
    }
    echo count($g), "\n";
}

class Holder {
    public $items = ['a' => 1, 'b' => 2, 'c' => ['d' => 3]];
    private $priv = ['x' => 1];
    function drop($k) { unset($this->items[$k]); unset($this->priv['x']); return [$this->items, $this->priv]; }
    function dropDeep() { unset($this->items['c']['d']); return $this->items; }
}

echo "-- unset of property dimensions\n";
for ($r = 0; $r < 2; $r++) {
    $h = new Holder;
    var_dump($h->drop('a'), $h->dropDeep());
    unset($h->items['b']);
    var_dump($h->items);
}

echo "-- ++/-- on dimensions\n";
for ($r = 0; $r < 2; $r++) {
    $a = ['i' => 1, 'f' => 1.5, 'n' => null, 's' => 'a', 'ns' => '5', 'big' => PHP_INT_MAX, 'small' => PHP_INT_MIN, 'b' => true, 'deep' => ['x' => ['y' => 10]]];
    var_dump($a['i']++, ++$a['i'], $a['i']--, --$a['i']);
    var_dump($a['f']++, --$a['f']);
    var_dump($a['n']++, $a['n']);
    $a['n'] = null;
    var_dump($a['n']--, $a['n']);
    var_dump($a['s']++, $a['s']);
    var_dump($a['ns']++, $a['ns']);
    var_dump($a['big']++, $a['big']);
    var_dump($a['small']--, $a['small']);
    var_dump($a['b']++, $a['b']);
    var_dump($a['deep']['x']['y']++, ++$a['deep']['x']['y'], $a['deep']['x']['y']);
    var_dump($a['missing']++);
    // (A missing key on the way, an undefined variable: #427.)
    var_dump($a['missing']);
    $k = 'i';
    for ($i = 0; $i < 3; $i++) {
        $a[$k]++;
        $counts[$i % 2] = ($counts[$i % 2] ?? 0) + 1;
    }
    var_dump($a['i'], $counts);
    // A copy is not touched.
    $b = ['p' => ['q' => 1]];
    $c = $b;
    $c['p']['q']++;
    var_dump($b['p']['q'], $c['p']['q']);
    // Through a reference, and an element that is a reference.
    $e = ['a' => ['b' => 1]];
    $ref = &$e;
    $ref['a']['b']++;
    $elem = &$e['a']['b'];
    $e['a']['b']++;
    var_dump($e, $elem);
    unset($ref, $elem);
    // Keys of other types.
    $d = [0 => 0, 1 => 1, '' => 2];
    $d[1.5]++;
    $d[true]++;
    $d[null]++;
    $d['1']++;
    var_dump($d);
    // Not arrays.
    $nul = null;
    $nul['k']++;
    var_dump($nul);
    $str = 'abc';
    try {
        $str[0]++;
    } catch (Error $e2) {
        echo get_class($e2), ': ', $e2->getMessage(), "\n";
    }
    $bag = new Bag;
    $bag->data = ['k' => 1];
    var_dump($bag['k']++);
    var_dump($bag->data);
    $obj = ['o' => new stdClass];
    try {
        $obj['o']['k']++;
    } catch (Error $e2) {
        echo get_class($e2), ': ', $e2->getMessage(), "\n";
    }
    $arr_in = ['x' => []];
    try {
        $arr_in['x']++;
    } catch (TypeError $e2) {
        echo get_class($e2), ': ', $e2->getMessage(), "\n";
    }
}

class Casty {
    function __toString(): string { return 'casty'; }
}

echo "-- casts\n";
$fh = fopen('php://memory', 'r');
$sources = [null, true, false, 0, -7, 42, 1.5, -0.0, 1e20, -1e20, NAN, INF, 0.1 + 0.2, '12', '12abc', 'abc', ' 12', '12 ', '1e3', '0x1A', '', '0', [], [1, 2], new stdClass, new Casty];
for ($r = 0; $r < 2; $r++) {
    foreach ($sources as $i => $v) {
        echo "$i:";
        var_dump((int) $v);
        var_dump((float) $v);
        var_dump((bool) $v);
        try {
            var_dump((string) $v);
        } catch (Error $e2) {
            echo get_class($e2), ': ', $e2->getMessage(), "\n";
        }
    }
    foreach ([null, true, 1, 1.5, 'x', [1, 'k' => 2], NAN, new Casty] as $v) {
        var_dump((array) $v);
    }
    var_dump(count((array) function () {}));
    $ref_src = 5;
    $ref = &$ref_src;
    var_dump((string) $ref, (int) $ref, (bool) $ref, (float) $ref, (array) $ref);
    unset($ref);
    var_dump((int) $undefined_cast);
    var_dump((string) $undefined_cast);
    var_dump((int) $fh > 0, (bool) $fh, (float) $fh > 0, str_starts_with((string) $fh, 'Resource id #'));
    // In a loop, a value of each type in turn.
    $t = 0;
    for ($i = 0; $i < 4; $i++) {
        $t += (int) ('1' . $i) + (int) ($i * 1.5) + (int) (bool) $i;
        $s = (string) $i . (string) ($i / 2);
    }
    var_dump($t, $s);
}

class Counter {
    public $n = 0;
    public int $typed = 0;
    public ?int $nullable = null;
    public float $fl = 1.5;
    public $str = 'a';
    public int $max = PHP_INT_MAX;
    public readonly int $ro;
    public private(set) int $asym = 0;
    private $secret = 0;
    protected $prot = 0;
    public int $hooked = 0 {
        set(int $v) { echo "hook set $v\n"; $this->hooked = $v; }
    }
    public $refd = 0;
    function __construct() { $this->ro = 1; }
    function bump() { $this->n++; ++$this->secret; $this->prot--; return [$this->n, $this->secret, $this->prot]; }
    function bumpAsym() { $this->asym++; return $this->asym; }
    function bumpRo() { $this->ro++; }
}

class Magic {
    private $data = ['m' => 1];
    function __get($k) { echo "__get($k)\n"; return $this->data[$k] ?? null; }
    function __set($k, $v) { echo "__set($k)\n"; $this->data[$k] = $v; }
}

echo "-- ++/-- on properties\n";
for ($r = 0; $r < 2; $r++) {
    $o = new Counter;
    var_dump($o->n++, ++$o->n, $o->n--, --$o->n);
    var_dump($o->typed++, $o->typed);
    var_dump($o->nullable++, $o->nullable);
    $o->nullable = null;
    var_dump($o->nullable--, $o->nullable);
    var_dump($o->fl++, $o->fl);
    var_dump($o->str++, $o->str);
    // (Past PHP_INT_MAX, an undefined property, a property of null: #427.)
    $o->max--;
    var_dump($o->max);
    try {
        $o->bumpRo();
    } catch (Error $e2) {
        echo get_class($e2), ': ', $e2->getMessage(), "\n";
    }
    try {
        $o->asym++;
    } catch (Error $e2) {
        echo get_class($e2), ': ', $e2->getMessage(), "\n";
    }
    var_dump($o->bumpAsym());
    var_dump($o->bump(), $o->bump());
    try {
        $o->secret++;
    } catch (Error $e2) {
        echo get_class($e2), ': ', $e2->getMessage(), "\n";
    }
    $o->hooked++;
    var_dump($o->hooked);
    $alias = &$o->refd;
    $o->refd++;
    var_dump($alias);
    unset($alias);
    $o->dyn ??= 0;
    $o->dyn++;
    var_dump($o->dyn);
    $m = new Magic;
    var_dump($m->m++);
    var_dump($m->m);
    $std = new stdClass;
    $std->c = 1;
    $std->c++;
    var_dump($std->c);
    // In a loop: the site's cache answers from the second step on.
    $t = 0;
    for ($i = 0; $i < 3; $i++) {
        $o->n++;
        $t += $o->n;
    }
    var_dump($t);
}

class Takes {
    function byRef(&$x) { $x = 'set'; return 'r'; }
    function byVal($x) { return $x; }
    function __call($m, $args) { return "__call $m " . count($args); }
}

class TakesToo extends Takes {
    function byRef(&$x) { $x = 'too'; return 'r2'; }
}

function make() { return [1]; }

echo "-- by-reference method arguments\n";
for ($r = 0; $r < 2; $r++) {
    $a = ['k' => 1];
    $t = new Takes;
    var_dump($t->byRef($a['k']), $a);
    var_dump($t->byRef($a['new']['deep']), $a);
    var_dump($t->byVal($a['k']), $t->byVal($a['nope'] ?? 'dflt'));
    var_dump($t->byVal($a['missing']));
    var_dump($t->magic($a['k']));
    $p = new stdClass;
    $p->v = 1;
    var_dump($t->byRef($p->v), $p->v);
    var_dump($t->byRef($p->fresh), $p->fresh);
    // A polymorphic site.
    foreach ([new Takes, new TakesToo, new Takes] as $x) {
        $b = ['k' => 0];
        var_dump($x->byRef($b['k']), $b['k']);
    }
    // A temporary's element to a by-reference parameter (a call's result
    // itself: #429).
    var_dump($t->byRef(make()[0]));
    $db = new PDO('sqlite::memory:');
    $st = $db->prepare('select ?');
    $bound = [];
    $st->bindParam(1, $bound['x']);
    $bound['x'] = "bound$r";
    $st->execute();
    var_dump($st->fetchColumn(), $bound);
    $nul = null;
    try {
        $nul->byRef($a['k']);
    } catch (Error $e2) {
        echo get_class($e2), ': ', $e2->getMessage(), "\n";
    }
}

class Its implements Iterator {
    private $i = 0;
    function __construct(private array $v) {}
    function current(): mixed { echo "current "; return $this->v[$this->i]; }
    function key(): mixed { echo "key "; return "k{$this->i}"; }
    function next(): void { echo "next "; $this->i++; }
    function rewind(): void { echo "rewind "; $this->i = 0; }
    // (Quiet: p8 calls it once more before the first element, #428.)
    function valid(): bool { return $this->i < count($this->v); }
}

class Agg implements IteratorAggregate {
    function __construct(private array $v) {}
    function getIterator(): Iterator { return new ArrayIterator($this->v); }
}

class GenAgg implements IteratorAggregate {
    function getIterator(): Generator { yield 'a' => 1; yield 'b' => 2; }
}

class Props {
    public $pub = 1;
    protected $prot = 2;
    private $priv = 3;
    public $unset_one = 4;
    function inside() { $out = []; foreach ($this as $k => $v) { $out[] = "$k=$v"; } return implode(',', $out); }
}

class PropsChild extends Props {
    private $own = 5;
    function child() { $out = []; foreach ($this as $k => $v) { $out[] = "$k=$v"; } return implode(',', $out); }
}

function gen() {
    $x = yield 1;
    echo "got ", var_export($x, true), "\n";
    yield 'k' => 2;
    return 3;
}

echo "-- foreach over objects\n";
for ($r = 0; $r < 2; $r++) {
    foreach (new Its([10, 20]) as $k => $v) {
        echo "$k=$v\n";
    }
    foreach (new Its([]) as $v) {
        echo "never\n";
    }
    echo "|\n";
    foreach (new Agg(['x' => 1, 'y' => 2]) as $k => $v) {
        echo "$k=$v ";
    }
    foreach (new GenAgg as $k => $v) {
        echo "$k=$v ";
    }
    $g = gen();
    foreach ($g as $k => $v) {
        echo "$k=$v ";
    }
    echo $g->getReturn(), "\n";
    $p = new Props;
    unset($p->unset_one);
    $p->dyn = 6;
    foreach ($p as $k => $v) {
        echo "$k=$v ";
    }
    echo "| ", $p->inside(), " | ", (new PropsChild)->child(), "\n";
    $ai = new ArrayIterator([1, 2, 3]);
    foreach ($ai as $k => $v) {
        if ($k == 0) {
            $ai[5] = 9;
        }
        echo "$k=$v ";
    }
    $ao = new ArrayObject(['p' => 1, 'q' => 2]);
    foreach ($ao as $k => $v) {
        echo "$k=$v ";
    }
    $empty = new stdClass;
    foreach ($empty as $v) {
        echo "never";
    }
    foreach ((function () { yield from [1, 2]; yield 3; })() as $k => $v) {
        echo "$k=$v ";
    }
    // An exception out of the iterator's methods.
    $bad = new class implements Iterator {
        function current(): mixed { return 1; }
        function key(): mixed { return 0; }
        function next(): void { throw new RuntimeException('from next'); }
        function rewind(): void {}
        function valid(): bool { return true; }
    };
    try {
        foreach ($bad as $v) {
            echo "v=$v ";
        }
    } catch (RuntimeException $e2) {
        echo get_class($e2), ': ', $e2->getMessage(), "\n";
    }
    $notit = new class implements IteratorAggregate {
        function getIterator(): Traversable { return new ArrayIterator([7]); }
    };
    foreach ($notit as $v) {
        echo "v=$v\n";
    }
}

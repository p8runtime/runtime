<?php
// The third batch of loop ports (#375) and their cold cases: `global $x`
// in a function (BindGlobal) and writes through a path of dimensions
// (AssignDimPath). Each case runs twice, so the second run goes through
// the caches the first filled.
// No `@`; the expected output is PHP 8.6's.

function counter() {
    global $count;
    $count++;
    return $count;
}

function read_g() {
    global $g;
    return $g;
}

function write_g($v) {
    global $g;
    $g = $v;
}

function undef_g() {
    global $never;
    var_dump($never);
}

function ref_g() {
    global $target;
    $target .= '+';
}

echo "-- global in functions\n";
include __DIR__ . '/generated_ports3.inc';
for ($r = 0; $r < 2; $r++) {
    var_dump(counter(), counter());
    write_g("w$r");
    var_dump(read_g(), $GLOBALS['g']);
    undef_g();
    var_dump(array_key_exists('never', $GLOBALS));
}

echo "-- \$GLOBALS writes, unset, rebinding\n";
for ($r = 0; $r < 2; $r++) {
    $GLOBALS['g'] = "set$r";
    var_dump(read_g());
    unset($GLOBALS['g']);
    var_dump(read_g());
    var_dump(isset($GLOBALS['g']), array_key_exists('g', $GLOBALS));
    // A global removed and made again (other globals after it move).
    unset($GLOBALS['count']);
    var_dump(counter(), counter());
    $GLOBALS['fresh' . $r] = $r;
}

echo "-- by-reference globals\n";
$source = 'a';
$GLOBALS['target'] = &$source;
for ($r = 0; $r < 2; $r++) {
    ref_g();
    var_dump($source);
    $other = 'b';
    $GLOBALS['target'] = &$other;
    ref_g();
    var_dump($other, $source);
    $GLOBALS['target'] = &$source;
}

echo "-- global of a main script variable\n";
$mainvar = 1;
function main_g() {
    global $mainvar;
    return ++$mainvar;
}
for ($r = 0; $r < 2; $r++) {
    var_dump(main_g(), $mainvar);
}

echo "-- dimension paths\n";
function paths($r) {
    $a = ['x' => ['y' => 1]];
    $a['x']['y'] = [$r];
    $a['x']['z'] = 'new';
    $a['n']['m'] = 2;
    $a[5][6][7] = 3;
    $a['5']['k'] = 4;
    $nul = ['p' => null];
    $nul['p']['q'] = 5;
    $copy = $a;
    $copy['x']['y'] = 'changed';
    var_dump($a, $copy, $nul);
    $v = ($a['r']['s'] = 'used');
    var_dump($v);
}
for ($r = 0; $r < 2; $r++) {
    paths($r);
}

echo "-- dimension paths: cold levels\n";
class AA implements ArrayAccess {
    public $data = [];
    function offsetExists($o): bool { return isset($this->data[$o]); }
    function offsetGet($o): mixed { echo "get $o\n"; return $this->data[$o] ?? null; }
    function offsetSet($o, $v): void { echo "set $o\n"; $this->data[$o] = $v; }
    function offsetUnset($o): void { unset($this->data[$o]); }
}
for ($r = 0; $r < 2; $r++) {
    $s = ['str' => 'abc', 'f' => false, 'i' => 3, 'o' => new AA, 'ref' => []];
    $s['o']['k'] = ['v' => 1];
    $s['o']['k']['w'] = 2;
    $s['f']['k'] = 1;
    try {
        $s['i']['k'] = 1;
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    try {
        $s['str']['k'] = 'z';
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    $s['str'][1] = 'Z';
    try {
        $s['str'][0][0] = 'z';
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    $inner = ['q' => 0];
    $s['ref'] = &$inner;
    $s['ref']['q'] = 'through';
    $s['ref']['new']['deep'] = 1;
    var_dump($inner, $s['f'], $s['str']);
    $k = 1.5;
    $s['x'][$k] = 'float';
    $s['x'][true] = 'bool';
    $s['x'][null] = 'null';
    var_dump($s['x']);
    $undefval = [];
    $undefval['a']['b'] = $nope;
    var_dump($undefval);
    $s['y'][$undefkey]['b'] = 1;
    var_dump($s['y']);
    $nul2 = null;
    $nul2['a']['b'] = 1;
    var_dump($nul2);
    $st = 'text';
    try {
        $st['a']['b'] = 1;
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}

echo "-- property dimension paths\n";
class Holder {
    public $data = ['a' => ['b' => 1]];
    public $none;
    public ?array $typed = null;
    public ?Holder $obj = null;
    public readonly array $ro;
    private array $hidden = [];
    public $str = 'xyz';
    public $aa;
    function fill($r) {
        $this->data['a']['c'] = $r;
        $this->data['n'] = [$r];
        $this->data['x']['y']['z'] = 'deep';
        $this->none['k'] = 'made';
        $this->hidden['h'][$r] = true;
        return $this->hidden;
    }
    function init() {
        $this->ro = [];
    }
}
for ($r = 0; $r < 2; $r++) {
    $h = new Holder;
    var_dump($h->fill($r), $h->data, $h->none);
    $h->data['a']['b'] = 'out';
    $v = ($h->data['q'] = 'used');
    var_dump($v, $h->data['a']);
    $copy = $h->data;
    $h->data['a']['b'] = 'again';
    var_dump($copy['a']['b'], $h->data['a']['b']);
    try {
        $h->typed['k'] = 1;
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    var_dump($h->typed);
    $h->init();
    try {
        $h->ro['k'] = 1;
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    try {
        $h->hidden['k'] = 1;
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    try {
        $h->str['k'] = 1;
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    $h->aa = new AA;
    $h->aa['k'] = 'via offsetSet';
    $inner = [];
    $h->data = &$inner;
    $h->data['through'] = 'ref';
    var_dump($inner);
    $h->data = [];
    $h->data[1.5]['k'] = 'float key';
    $h->data[$undefk] = 'undef key';
    var_dump($h->data);
}

echo "-- static calls choosing by-reference\n";
class S {
    static function byref(&$x) { $x .= '!'; return $x; }
    static function byval($x) { $x .= '?'; return $x; }
    static function pick(string $how, $v) {
        // (An argument that is no variable: each is sent on its own.)
        return $how === 'ref' ? static::byref($v, $how . '') : static::byval($v, $how . '');
    }
    static function undef($how) {
        return static::byval($nope, $how . '');
    }
}
class T extends S {
    static function byref(&$x) { $x .= '#'; return $x; }
}
for ($r = 0; $r < 2; $r++) {
    $a = 'a';
    $m = 'byref';
    var_dump(S::$m($a), $a);
    $b = 'b';
    $m = 'byval';
    var_dump(S::$m($b), $b);
    var_dump(S::pick('ref', 'p'), T::pick('ref', 'q'), T::pick('val', 'v'), T::undef('u'));
    $n = 'byref';
    var_dump(S::$n($undefvar));
    var_dump($undefvar);
}

echo "-- new with spread arguments\n";
class Point {
    public function __construct(public $x = 0, public $y = 0, ...$rest) {
        echo 'Point(', $x, ', ', $y, ') rest ', json_encode($rest), "\n";
    }
}
class Needs {
    public function __construct($a, $b) {}
}
class NoCtor {}
for ($r = 0; $r < 2; $r++) {
    $args = [1, 2];
    $p = new Point(...$args);
    $p = new Point(...[3, 4, 5, 6]);
    $p = new Point(...[]);
    $p = new Point(...[5 => 'a', 9 => 'b']);
    $p = new Point(...['y' => 7]);
    $p = new Point(...['x' => 1, 'extra' => 2]);
    $p = new Point(8, ...['y' => 9]);
    $v = 'by ref';
    $list = [&$v, 2];
    $p = new Point(...$list);
    try {
        $p = new Point(...[1, 'x' => 2]);
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    try {
        new Needs(...[1]);
    } catch (ArgumentCountError $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    try {
        new Needs(...['c' => 1]);
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    $n = new NoCtor(...[1, 2]);
    var_dump($n instanceof NoCtor);
    $gen = (function () { yield 1; yield 2; })();
    $p = new Point(...$gen);
}

echo "-- appends at the end of a path\n";
class Bag {
    public $items;
    public array $typed = [];
    public ?array $maybe = null;
    public $full = [PHP_INT_MAX => 'last'];
    function add($v) {
        $this->items[] = $v;
        $this->typed[] = $v;
        $this->maybe[] = $v;
        $this->typed['by'][] = $v;
        try {
            $this->full[] = $v;
        } catch (Error $e) {
            echo get_class($e), ': ', $e->getMessage(), "\n";
        }
        return count($this->items);
    }
}
for ($r = 0; $r < 2; $r++) {
    $bag = new Bag;
    var_dump($bag->add('a'), $bag->add('b'));
    var_dump($bag->items, $bag->typed, $bag->maybe);
    $k = 'key';
    $a = [];
    $a[$k][] = 1;
    $a[$k][] = 2;
    $a[$k]['in'][] = 3;
    $a['x' . $r][] = 'made';
    $a[$k . 'f'] = false;
    $a[$k . 'f'][] = 'from false';
    $a[$k . 'm'] = [PHP_INT_MAX => 0];
    try {
        $a[$k . 'm'][] = 'no room';
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    $a[$k . 's'] = 'str';
    try {
        $a[$k . 's'][] = 'x';
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    var_dump($a);
    $v = ($a[$k][] = 'used');
    var_dump($v);
}

echo "-- method calls: native methods, classes a site has not seen\n";
interface Shape { function area(); }
class Sq implements Shape { function __construct(public $s) {} function area() { return $this->s * $this->s; } }
class Ci implements Shape { function area() { return 3; } private function hidden() { return 'h'; } }
class Magic { function __call($name, $args) { return "$name(" . implode(',', $args) . ")"; } }
class Sub extends ArrayObject { function count(): int { return 99; } }
for ($r = 0; $r < 2; $r++) {
    $t = 0;
    foreach ([new Sq(2), new Ci, new Sq(3), new Ci] as $shape) {
        $t += $shape->area();
    }
    var_dump($t);
    foreach ([new ArrayObject([1, 2]), new ArrayIterator([1, 2, 3]), new Sub([1]), new SplStack] as $c) {
        var_dump($c->count());
    }
    foreach ([new Magic, new Ci] as $m) {
        try {
            var_dump($m->hidden(1, 2));
        } catch (Error $e) {
            echo get_class($e), ': ', $e->getMessage(), "\n";
        }
        try {
            var_dump($m->nope());
        } catch (Error $e) {
            echo get_class($e), ': ', $e->getMessage(), "\n";
        }
    }
    $d = new DateTime('2020-01-02 03:04:05', new DateTimeZone('UTC'));
    var_dump($d->format('Y-m-d'), $d->getTimestamp());
    $none = null;
    try {
        $none->area();
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}

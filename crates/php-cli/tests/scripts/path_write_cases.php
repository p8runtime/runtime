<?php
// Writes through a path of keys (`$a[k][j] = v`, `$o->p[k][] = v`,
// `unset($a[k][j])`): what is written, what a copy made before keeps,
// what a reference sees, for arrays held by variables, statics, constants
// and properties, and during foreach. Then when the temporaries of the
// statements around a write are released: a destructor tells.

class D {
    public function __construct(public string $name) {}
    public function __destruct() { echo "destruct {$this->name}\n"; }
}

function show(string $label, mixed $v): void {
    echo $label, ': ', json_encode($v), "\n";
}

// Nested writes: each kind of key at each level, levels made on the way.
$a = [];
$a['x']['y'] = 1;
$a['x']['z']['w'] = 2;
$a[3][4] = 'int keys';
$a['5'][' 6'] = 'a numeric string is an int key, one with a space is not';
$a['007']['-0'] = 'not canonical: strings';
$a['-8']['9223372036854775807'] = 'negative, and the largest int';
$a['x']['9223372036854775808'] = 'one past it: a string';
$a[''][''] = 'empty';
$a['x']['y'] = 'replaced';
$a[true][false] = 'bool keys';
show('nested', $a);
var_dump(array_keys($a), array_keys($a['x']), array_keys($a[-8]));

// A level that is null is made an array; the next key after int keys.
$n = null;
$n['a']['b'][] = 1;
$n['a']['b'][] = 2;
$n['a']['b'][10] = 3;
$n['a']['b'][] = 4;
$n['a']['b']['s'] = 5;
$n['a']['b'][] = 6;
$n['a']['c'][-5] = 7;
$n['a']['c'][] = 8;
show('from null', $n);

// A list that takes a new int key at its end, past its end, a string key.
$l = [[10, 11, 12]];
$l[0][3] = 13;
show('list, next', $l);
$l[0][5] = 15;
show('list, a gap', $l);
$l[0][] = 16;
$l[0]['k'] = 'now a hash';
$l[0][4] = 14;
$l[0][] = 17;
show('list, then a hash', $l);
$big = [[1, 2]];
$big[0][1000000] = 3;
$big[0][] = 4;
show('far key', $big);

// A hash that takes a new int key: the next key moves.
$h = ['p' => ['a' => 1]];
$h['p'][7] = 2;
$h['p'][] = 3;
$h['p'][2] = 4;
$h['p'][] = 5;
$h['p'][-1] = 6;
show('hash, int keys', $h);

// Many new keys in one array, past the size a small table has.
$m = ['t' => []];
for ($i = 0; $i < 40; $i++) {
    $m['t']["k$i"] = $i;
    $m['t'][$i * 3] = -$i;
}
echo 'many: ', count($m['t']), ' ', $m['t']['k39'], ' ', $m['t'][117], ' ', array_key_last($m['t']), "\n";
$m['t'][] = 'next';
echo 'many, next key: ', array_key_last($m['t']), "\n";

// A copy made before a write keeps what it had, at every level.
$orig = ['a' => ['b' => ['c' => 1]], 'l' => [1, 2, 3]];
$copy = $orig;
$inner = $orig['a'];
$deep = $orig['a']['b'];
$orig['a']['b']['c'] = 2;
$orig['a']['b']['new'] = 3;
$orig['a']['n']['m'] = 4;
$orig['l'][] = 4;
$orig['l'][0] = 'first';
show('written', $orig);
show('copy', $copy);
show('inner copy', $inner);
show('deep copy', $deep);
$copy['a']['b']['c'] = 'in the copy';
unset($copy['l'][1]);
show('copy written', $copy);
show('original after', $orig);

// The result of a write is the value written.
$r = [];
$x = $r['a']['b'] = [1, 2];
$x[] = 3;
show('result', [$x, $r]);
$y = $r['a']['c'][] = 'pushed';
show('result of a push', [$y, $r]);
echo 'result in an expression: ', ($r['n']['m'] = 20) + 1, ' ', strlen($r['s']['t'] = 'four'), "\n";
$o = new stdClass;
$o->p = [];
$z = $o->p['k']['j'] = ['v'];
$z[] = 'w';
show('result of a property write', [$z, $o->p]);
var_dump($o->p['q'][] = 1.5);

// unset: a key that is there, one that is not, in an array a copy shares.
$u = ['a' => ['b' => 1, 'c' => 2], 'l' => [1, 2, 3]];
$keep = $u;
unset($u['a']['zz']);
unset($u['zz']['b']);
unset($u['l'][7]);
show('unset, nothing there', $u);
unset($u['a']['b']);
unset($u['l'][2]);
unset($u['l'][0]);
show('unset', $u);
show('unset, the copy', $keep);
$u['l'][] = 'after the unsets';
show('push after unset', $u);
$own = ['a' => ['b' => ['c' => 1, 'd' => 2]]];
unset($own['a']['b']['c'], $own['a']['x']['c'], $own['a']['b']['c']);
show('unset, its own', $own);
try {
    unset($own['a']['b']['d']['e']);
} catch (Error $e) {
    echo 'unset below a scalar: ', $e->getMessage(), "\n";
}
show('after it', $own);

// References: to a variable, to an element, to a level, to the root.
$base = ['k' => ['v' => 1]];
$ref = &$base;
$ref['k']['w'] = 2;
show('through a reference', $base);
$el = &$base['k']['v'];
$copy = $base;
$base['k']['v'] = 'written through the element';
show('element reference', [$el, $copy['k']['v']]);
unset($el);
$level = &$base['k'];
$level['x']['y'] = 3;
$base['k']['x']['z'] = 4;
show('level reference', [$base, $level]);
unset($level);
$snap = $base;
$base['k']['x']['z'] = 5;
show('after the reference is gone', [$base['k']['x'], $snap['k']['x']]);
function by_ref(array &$p): void {
    $p['in']['fn'][] = 1;
    unset($p['k']['w']);
}
by_ref($base);
show('by-reference parameter', $base);
$self = ['p' => [0]];
$self['p'][0] = &$self;
$self['p'][0] = 'the root replaced';
show('element bound to the root', $self);

// An array a static holds, one a constant holds, one in a property.
function counted(string $k): array {
    static $seen = ['n' => []];
    $before = $seen;
    $seen['n'][$k][] = count($seen['n']);
    return [$before, $seen];
}
counted('a');
counted('b');
show('static', counted('a'));
const TABLE = ['rows' => [[1, 2], [3, 4]], 'name' => 'table'];
class K {
    const C = ['x' => ['y' => 1]];
    public static array $s = ['x' => ['y' => 1]];
    public array $p = self::C;
    public $q;
    function write(): void {
        $this->p['x']['y'] = 2;
        $this->p['x'][] = 3;
        $this->q['made']['here'] = 4;
        unset($this->p['x']['nothing']);
    }
}
$t = TABLE;
$t['rows'][1][0] = 'changed';
$t['rows'][] = [5, 6];
unset($t['rows'][0][1]);
show('from a constant', $t);
show('the constant', TABLE);
$k = new K;
$k2 = clone $k;
$k->write();
K::$s['x']['z'] = 5;
$c = K::C;
$c['x']['y'] = 'local';
show('property', [$k->p, $k->q, $k2->p, K::C, K::$s, $c]);
$held = $k->p;
$k->p['x']['y'] = 6;
unset($k->p['x'][0]);
show('property, a copy held', [$held, $k->p]);

// Writes while a foreach walks the array (by value: its own copy; by
// reference: the array itself).
$f = ['a' => [1], 'b' => [2], 'c' => [3]];
foreach ($f as $key => $row) {
    $f[$key][] = $row[0] * 10;
    $f['new' . $key]['n'] = $key;
    unset($f['c'][0]);
}
show('foreach by value', $f);
$g = ['a' => [1], 'b' => [2]];
foreach ($g as $key => &$row) {
    $row['seen'] = true;
    $g[$key]['also'] = $key;
    if ($key === 'a') {
        $g['z']['added'] = 1;
    }
}
unset($row);
show('foreach by reference', $g);
$w = new K;
foreach ($w->p as $key => $row) {
    $w->p[$key]['y'] = 'in the loop';
    $w->p['more'][$key] = $row;
}
show('foreach over a property', $w->p);

// The pattern that keeps a hook table: asked for with isset(), written
// two levels down, read again.
class Hooks {
    public $callbacks = [];
    public $priorities = [];
    function add(string $name, int $priority): void {
        $existed = isset($this->callbacks[$priority]);
        $this->callbacks[$priority][$name] = ['function' => $name, 'accepted_args' => 1];
        if (!$existed && count($this->callbacks) > 1) {
            ksort($this->callbacks, SORT_NUMERIC);
        }
        $this->priorities = array_keys($this->callbacks);
    }
}
$hooks = new Hooks;
$earlier = [];
foreach ([['init', 10], ['late', 20], ['early', 1], ['init2', 10]] as [$name, $priority]) {
    $earlier[] = $hooks->callbacks;
    $hooks->add($name, $priority);
}
show('hooks', [$hooks->callbacks, $hooks->priorities]);
show('hooks, the copies', array_map('count', $earlier));

// When a temporary goes. The array an isset() looked into is let go
// before the next statement: the object in it is destroyed by the write
// that replaces the property, not later.
class Box {
    public $p;
    public $q;
    public static $s;
    public $n = 0;
}
function isset_then_assign(Box $o, string $s): void {
    $x = isset($o->p[5]);
    $o->p = null;
    echo "after the property is replaced\n";
    $o->q = strlen($s);
    echo "end of isset_then_assign\n";
}
$box = new Box;
$box->p = [new D('in p')];
isset_then_assign($box, 'abc');

function isset_then_path(Box $o, string $s): void {
    $x = isset($o->p['k']);
    $o->p = null;
    echo "after the property is replaced\n";
    $o->q['a'][$s] = strlen($s);
    echo "end of isset_then_path\n";
}
$box->p = ['k' => new D('in p again')];
$box->q = null;
isset_then_path($box, 'abc');

function isset_then_dim(array $a, array $t, string $s): void {
    $x = isset($t['k']['j']);
    $t = null;
    echo "after the array is replaced\n";
    $a[$s] = [strlen($s)];
    $a[$s][$s] = strlen($s) + 1;
    echo "end of isset_then_dim\n";
}
isset_then_dim([], ['k' => ['j' => new D('in t')]], 'abc');

function isset_then_steps(Box $o, string $s): void {
    $x = isset($o->p['k']);
    $o->p = null;
    echo "after the property is replaced\n";
    $o->n += strlen($s);
    $o->n++;
    Box::$s = strlen($s);
    Box::$s .= $s;
    $o->q['a'][$s] .= $s;
    $o->q['a'][$s . $s] ??= $s;
    echo "end of isset_then_steps\n";
}
$box->p = ['k' => new D('in p, a third time')];
isset_then_steps($box, 'abc');
show('the box', [$box->q, $box->n, Box::$s]);

// The value of a write nobody keeps, and the object written into, are
// let go in the order PHP lets them go.
class Sink implements ArrayAccess {
    public function offsetExists(mixed $k): bool { return false; }
    public function offsetGet(mixed $k): mixed { return null; }
    public function offsetSet(mixed $k, mixed $v): void { echo "offsetSet ", json_encode($k), "\n"; }
    public function offsetUnset(mixed $k): void {}
}
class Holder {
    public $p = [];
    public function __construct(public string $name) {}
    public function __destruct() { echo "destruct holder {$this->name}\n"; }
}
function holder(string $name): Holder {
    return new Holder($name);
}
function discarded(): void {
    $s = new Sink;
    $s['k'] = new D('given to a sink');
    echo "after the sink\n";
    $a = [];
    $a['k']['j'] = new D('kept in an array');
    echo "after the array write\n";
    $a['k']['j'] = new D('replaces it');
    echo "after the second write\n";
    holder('temporary')->p['k'][] = new D('in a temporary holder');
    echo "after the temporary holder\n";
    $h = new Holder('local');
    $h->p['k']['j'] = new D('in a local holder');
    $h->p['k']['j'] = null;
    echo "end of discarded\n";
}
discarded();
echo "done\n";

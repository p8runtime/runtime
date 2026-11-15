<?php
// Array literals with literal keys and values only known when they run:
// built as a copy of the literal's shape with those values written in.
// Order, keys, copy-on-write and warnings must be what building the
// array element by element gives.

class D {
    public function __construct(public int $n) {}
    public function __destruct() { echo "destruct {$this->n}\n"; }
}

class M {
    public $p = 1;
    public function me() { return ['self' => $this, 'p' => $this->p]; }
}

function f($x) { return $x * 10; }

function build($i) {
    return ['a' => $i, 'b' => 2, 'c' => $i + 1, 'd' => [1, 2], 'e' => ['n' => $i]];
}

// Key order, known values in place, nested literals.
var_dump(build(1));

// The shape is never written: each build is a fresh array.
$all = [];
for ($i = 0; $i < 3; $i++) {
    $a = ['a' => $i, 'b' => 2];
    $a['b']++;
    $a['z'] = $i;
    $a[] = 'next';
    $all[] = $a;
}
var_dump($all);
$t = ['a' => $i, 'b' => 2];
var_dump($t);

// A copy of a built literal is separated at its write.
$u = $t;
$u['a'] = 'changed';
var_dump($t['a'], $u['a']);

// Int-like string keys are ints; others stay strings.
$x = 'X';
var_dump(['5' => $x, '05' => $x, '-0' => $x, '-7' => $x, ' 1' => $x, '9223372036854775808' => $x]);

// Int keys: holes, the next free index (after negative keys too).
$p = [0 => $x, 1 => $x];
$p[] = 'p';
var_dump($p);
$h = [3 => $x, 1 => $x];
$h[] = 'h';
var_dump($h);
$n = [-5 => $x, -3 => $x];
$n[] = 'n';
var_dump($n);

// Duplicate keys: the last value, at the first key's position.
$y = 'Y';
var_dump(['a' => $x, 'b' => 1, 'a' => $y]);
var_dump(['1' => $x, 1 => $y, 'k' => 3]);
var_dump(['a' => 1, 'a' => $y]);
var_dump(['a' => $y, 'b' => 1, 'a' => 2]);
var_dump(['1' => $y, 'k' => $x, 1 => 3]);

// Spread and references stay element by element.
$s = ['q' => 1];
var_dump(['a' => $x, ...$s]);
$r = 1;
$ref = ['a' => &$r, 'b' => $x];
$r = 2;
var_dump($ref);
$list = [&$r, 'k' => $x];
$r = 3;
var_dump($list);

// A variable bound by reference goes in as its value.
$v = 1;
$alias = &$v;
$byval = ['k' => $alias, 'j' => 2];
$v = 99;
var_dump($byval);

// The same variable twice, the array's own variable, a call, `$this`.
$w = ['a' => $x, 'b' => $x];
$w = ['old' => $w, 'n' => 1];
var_dump($w);
var_dump(['call' => f(3), 'k' => 1, 'v' => $x]);
$m = (new M())->me();
var_dump($m['p'], $m['self'] instanceof M);

// Undefined variables warn in element order, and are null.
var_dump(['a' => $undef1, 'b' => 1, 'c' => $undef2]);

// A null key is still the empty string, with its deprecation.
var_dump([null => $x, 'b' => 1]);

// Objects made in an element go when the array goes, not later.
echo "before\n";
$o = ['o' => new D(1), 'n' => 2, 'p' => new D(2)];
echo "built\n";
unset($o);
echo "after\n";

// A generator's yield as an element's value.
function gen() {
    $got = ['first' => yield 1, 'k' => 2, 'second' => yield 2];
    var_dump($got);
}
$g = gen();
$g->current();
$g->send('A');
$g->send('B');

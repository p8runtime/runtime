<?php
// Temporaries consumed by the instruction that reads them: values built
// into arrays and properties, returned, reused in one expression, and
// when objects held only by temporaries are destroyed.
class D {
    public $p; public $q = [];
    public function __construct(public string $n) { echo "make {$this->n}\n"; }
    public function __destruct() { echo "free {$this->n}\n"; }
    public function name() { return $this->n; }
}
function g($x) { return $x . '!'; }
function f($a, $b) { return "$a|$b"; }
function arr($x) { return [g($x), g($x) . g($x), strtoupper($x), [$x, g($x)]]; }
function ret_var() { $v = [1, 2, new D('ret var')]; return $v; }
function ret_temp($x) { return g($x) . g($x); }
function ret_ref() { static $s = 'static'; $r = &$s; return $r; }
function &ret_by_ref() { static $s = ['by ref']; return $s; }

echo f(g('x'), g('x')), "\n";
$a = 'a'; $b = 'b'; $c = 'c';
echo $a . $b . $c . g($a) . $a, "\n";
var_dump(arr('y'));
$t = true ? g('t') : g('f'); $u = null ?? g('n'); $w = g('w') ?: 'empty';
var_dump($t, $u, $w, match (g('m')) { 'm!' => g('matched'), default => 'no' });
[$p, [$q, $r]] = [g('p'), [g('q'), g('r')]];
['k' => $k] = ['k' => g('k')];
var_dump($p, $q, $r, $k);
$o = new D('holder');
$o->p = new D('in property');
$o->p = g('replaced');
echo "after replace\n";
$o->q = [new D('in array prop')];
$o->q = [];
echo "after array prop\n";
$o->p = $o->q = [g('chained')];
var_dump($o->p, $o->q);
// (One object per array: several are freed in reverse order, #230.)
$list = [g('x'), new D('elem 1'), g('y')];
echo "built list\n";
$list = null;
echo "after list\n";
$keyed = ['a' => new D('keyed a'), 'b' => [new D('nested')]];
unset($keyed['a']);
echo "after unset keyed\n";
$keyed = null;
echo "after keyed\n";
var_dump(count(ret_var()));
echo "after ret var\n";
var_dump(ret_temp('rt'), ret_ref());
$br = &ret_by_ref(); $br[] = 'more'; var_dump(ret_by_ref());
$s = 'base'; $copy = [$s, $s . 'x']; $s .= '!'; var_dump($copy, $s);
$obj = new D('same'); $pair = [$obj, $obj]; $obj = null; echo "pair still holds\n"; $pair = null; echo "after pair\n";
for ($i = 0; $i < 3; $i++) { $tmp = [new D("loop $i")]; }
echo "after loop\n";
$tmp = null;
echo (new D('chain'))->name(), "\n";
echo "end\n";

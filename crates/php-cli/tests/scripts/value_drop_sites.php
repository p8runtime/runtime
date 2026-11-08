<?php
// Values let go of outside a register write: a property, a static, a
// global, an array element or a reference overwritten; an argument's
// copy; a closure's captured variable; a generator's locals; an object's
// properties when it goes. The destructor runs where PHP runs it: at the
// overwrite, or when the holder goes.
class D {
    public $n;
    function __construct($n) { $this->n = $n; }
    function __destruct() { echo "~{$this->n}\n"; }
}
#[AllowDynamicProperties]
class Box {
    public $p;
    public static $s;
    function __destruct() { echo "~Box\n"; }
}

echo "-- property overwritten\n";
$b = new Box;
$b->p = new D('p0');
$b->p = new D('p1');
echo "set p1\n";
$b->p = 1;
echo "set int\n";
$b->p = [new D('p2')];
$b->p = "str";
echo "after property\n";

echo "-- static property overwritten\n";
Box::$s = new D('s0');
Box::$s = new D('s1');
echo "set s1\n";
Box::$s = null;
echo "after static\n";

echo "-- global overwritten from a function\n";
$g = new D('g0');
function setg() { global $g; $g = new D('g1'); echo "set g1\n"; $g = 2; }
setg();
echo "after global\n";

echo "-- array element overwritten, pushed over, unset\n";
$a = [new D('a0'), 'k' => new D('a1')];
$a[0] = new D('a2');
echo "set a2\n";
$a['k'] = 1;
echo "set k\n";
$a[0] = null;
echo "after element\n";

echo "-- a reference's value overwritten\n";
$r = new D('r0');
$ref = &$r;
$ref = new D('r1');
echo "set r1\n";
$r = 3;
echo "after reference\n";

echo "-- argument copies\n";
function take($x) { echo "in take {$x->n}\n"; }
take(new D('arg0'));
echo "after take\n";
$keep = new D('arg1');
take($keep);
echo "after take kept\n";

echo "-- closure capture\n";
$c = function () use ($keep) { return $keep->n; };
$keep = null;
echo "keep cleared, closure holds\n";
echo $c(), "\n";
$c = null;
echo "after closure\n";

echo "-- generator locals\n";
function gen() { $l = new D('gen0'); yield 1; yield 2; echo "gen end\n"; }
foreach (gen() as $v) { echo "yielded $v\n"; }
echo "after generator\n";
$g2 = gen();
$g2->current();
$g2 = null;
echo "after abandoned generator\n";

echo "-- object with properties goes\n";
$o = new Box;
$o->p = new D('o0');
$o->dyn = new D('o1');
$o->list = [new D('o2'), 'x' => new D('o3')];
$o = null;
echo "after object\n";

echo "-- string concatenation temporaries\n";
function name(D $d) { return $d->n; }
$s = "a" . name(new D('c0')) . "b" . name(new D('c1'));
echo "$s\n";
echo "after concat\n";

echo "-- compound assignment on an element holding an object\n";
$ca = [new D('ca0')];
$ca[0] = (int) 1;
$ca[0] += 1;
echo "after compound {$ca[0]}\n";

echo "-- list() destructuring over objects\n";
[$x, $y] = [new D('l0'), new D('l1')];
[$x, $y] = [1, 2];
echo "after list\n";

echo "-- swap through a temporary\n";
$m = new D('m0');
$n = new D('m1');
[$m, $n] = [$n, $m];
echo "swapped {$m->n} {$n->n}\n";
$m = $n = null;
echo "end\n";

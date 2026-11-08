<?php
// The last handle of an array goes: its elements are released in order
// (zend_array_destroy walks forward), a destructor runs for each object
// the array held alone, nested arrays and objects go with it, and a
// shared array (another handle left) frees nothing. Packed lists and
// string-keyed hashes alike; a string key's handle goes with its entry.
class D {
    public $n;
    function __construct($n) { $this->n = $n; }
    function __destruct() { echo "~{$this->n}\n"; }
}

echo "-- packed list, overwritten\n";
$a = [new D('p0'), 1, "s", new D('p1'), 2.5, null, new D('p2')];
$a = 0;
echo "after packed\n";

echo "-- hashed, unset\n";
$h = ['x' => new D('h0'), 'y' => 1, 'z' => new D('h1'), 7 => new D('h2'), 'w' => 'str'];
unset($h);
echo "after hashed\n";

echo "-- nested arrays and an object in an object\n";
$n = [[new D('n0'), [new D('n1')]], 'k' => ['deep' => [[new D('n2')]]]];
$n = null;
echo "after nested\n";

echo "-- shared: the second handle keeps everything\n";
$s = [new D('s0'), new D('s1')];
$t = $s;
$s = null;
echo "s gone, t holds\n";
$t = null;
echo "after shared\n";

echo "-- an element shared outside the array survives\n";
$o = new D('e1');
$e = [new D('e0'), $o, new D('e2')];
$e = [];
echo "array gone, e1 lives\n";
$o = null;
echo "after element\n";

echo "-- a reference inside an array\n";
$r = new D('r0');
$ra = [&$r, new D('r1')];
$ra = null;
echo "array gone, r0 lives\n";
$r = null;
echo "after reference\n";

echo "-- large list (past the pooled sizes)\n";
$big = [];
for ($i = 0; $i < 40; $i++) { $big[] = $i; }
$big[] = new D('b0');
for ($i = 0; $i < 40; $i++) { $big[] = "s$i"; }
$big[] = new D('b1');
$big = null;
echo "after large\n";

echo "-- a large hash with string keys\n";
$bh = [];
for ($i = 0; $i < 40; $i++) { $bh["key$i"] = $i; }
$bh['o0'] = new D('bh0');
$bh['o1'] = new D('bh1');
unset($bh['key3'], $bh['o0']);
echo "unset o0\n";
$bh = null;
echo "after large hash\n";

echo "-- returned from a function, dropped as a temporary\n";
function mk() { return [new D('t0'), 'x' => new D('t1')]; }
mk();
echo "after temporary\n";
count(mk());
echo "after count\n";

echo "-- the array held by an object that goes\n";
class H { public $items; function __destruct() { echo "~H\n"; } }
$hh = new H;
$hh->items = [new D('i0'), new D('i1')];
$hh = null;
echo "after holder\n";

echo "-- foreach over a temporary list of objects\n";
foreach ([new D('f0'), new D('f1')] as $v) { echo "body {$v->n}\n"; }
echo "after foreach\n";
$v = null;
echo "end\n";

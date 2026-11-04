<?php
// Destructor order: objects in arrays (freed, unset, reassigned, nested,
// spliced), object properties, function locals, objects a destructor
// frees itself, a destructor that throws, and the end of the script.
// Object ids show which handle each object gets, as PHP reuses them.
class D {
    function __construct(public $n) {}
    function __destruct() { echo "d{$this->n}#", spl_object_id($this), " "; }
}
echo "array: "; $a = [new D(1), new D(2), new D(3)]; $a = null; echo "\n";
echo "unset: "; $b = ['x' => new D(1), 'y' => new D(2), 'z' => new D(3)]; unset($b); echo "\n";
echo "nested: "; $c = [[new D(1), new D(2)], [new D(3), new D(4)]]; $c = null; echo "\n";
echo "deeper: "; $n = ['k' => [[new D('a')], 'b' => new D('b')], 'c' => new D('c')]; unset($n['k']); echo "| "; $n = null; echo "\n";
class Holder {
    public $p1; public $p2; public $p3;
    function __construct() { $this->p1 = new D('p1'); $this->p2 = new D('p2'); $this->p3 = new D('p3'); }
}
echo "props: "; $h = new Holder; echo "h#", spl_object_id($h), " "; $h = null; echo "\n";
echo "plain props: "; $o = new stdClass; $o->a = new D('a'); $o->b = [new D('b1'), new D('b2')]; $o->c = new D('c'); $o = null; echo "\n";
echo "locals: "; function f() { $x = new D('x'); $y = new D('y'); $z = new D('z'); } f(); echo "\n";
echo "reassign: "; $r = [new D(1), new D(2)]; $r = [new D(3)]; echo "now#", spl_object_id($r[0]), "\n";
echo "splice: "; $q = [new D(1), new D(2), new D(3)]; array_splice($q, 1); echo "| "; $q = null; echo "\n";
echo "ids reused: "; $i1 = new D('i1'); $i2 = new D('i2'); echo spl_object_id($i1), spl_object_id($i2), "\n";

// A destructor that frees objects: theirs run inside it, at that point.
class Parent_ {
    public $kids;
    function __construct() { $this->kids = [new D('k1'), new D('k2')]; }
    function __destruct() { echo "parent-start "; $this->kids = null; echo "parent-end "; }
}
echo "inside: "; $pp = [new Parent_, new D('after')]; $pp = null; echo "\n";

// A destructor that throws: the others in the same array still run.
class Thrower { function __destruct() { echo "thrower "; throw new RuntimeException('from dtor'); } }
echo "throws: ";
try { $t = [new D('t1'), new Thrower, new D('t2')]; $t = null; echo "not here "; } catch (RuntimeException $e) { echo "caught ", $e->getMessage(), " "; }
echo "\n";

echo "end: ";
$e1 = new D('e1'); $e2 = new D('e2'); $arr = [new D('a1'), new D('a2')]; $e3 = new D('e3');
$kept = new Holder;

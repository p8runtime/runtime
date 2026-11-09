<?php
// The object store (#396): ids are reused most recently freed first, and
// the objects a script leaves alive are destructed at its end in id order
// (zend_objects_store_call_destructors walks the handles), not in the
// order they were made.
class D {
    public $self;
    function __construct(public $n) {}
    function __destruct() { echo "destruct {$this->n} (id ", spl_object_id($this), ")\n"; }
}
class P { public $x = 1; }

$a = new D('a');          // 1
$p = new P;               // 2
$b = new D('b');          // 3
$c = new D('c');          // 4
$q = new P;               // 5
unset($a);                // 1 free
unset($q);                // 5 free
$d = new D('d');          // 5 (most recently freed)
$e = new D('e');          // 1
echo "d=", spl_object_id($d), " e=", spl_object_id($e), "\n";

// A destructor's own id waits for it to run; then it is the next one taken.
function scope() {
    $t = new D('t');
    echo "t=", spl_object_id($t), "\n";
}
scope();
$f = new P;
echo "f=", spl_object_id($f), "\n";

// Many short-lived objects: each takes the id the last one gave back.
$ids = [];
for ($i = 0; $i < 5; $i++) {
    $o = new P;
    $ids[] = spl_object_id($o);
    unset($o);
}
echo implode(',', $ids), "\n";

// A cycle keeps them for the store walk at the end; so does an array.
$b->self = $b;
$d->self = $d;
$keep = [$e, $c];
unset($b, $c, $d, $e);
// WeakReference and WeakMap see the ones that went, and not the others.
$gone = new P;
$w = WeakReference::create($gone);
$m = new WeakMap;
$m[$gone] = 'gone';
$m[$f] = 'f';
unset($gone);
var_dump($w->get(), count($m));
$again = new P;
var_dump($w->get(), spl_object_id($again));
echo "end\n";

<?php
// A method called on a value that is in no variable: `$this->f()`,
// `(new A)->f()`, a call's result. The object lives as long as the call
// needs it, and is released when PHP releases it.

class Node {
    public $n;
    public $log = [];
    function __construct($n) { $this->n = $n; }
    function __destruct() { echo "destruct {$this->n}\n"; }
    function me() { return $this; }
    function next() { return new Node($this->n + 1); }
    function add($x) { $this->n += $x; return $this; }
    function bump(&$x) { $x += $this->n; return $this; }
    function total($a, $b) { return $this->add($a)->add($b)->n; }
    function twice(&$x) { $this->bump($x); $this->bump($x); return $x; }
    function name() { return "node {$this->n}"; }
}

// 1. `$this->f()` with and without arguments, by value and by reference.
$a = new Node(1);
for ($i = 0; $i < 3; $i++) {
    $v = $i;
    echo $a->total($i, 10), ' ', $a->twice($v), " $v\n";
}

// 2. A new object: destroyed when the call that used it is over, before
// the next statement.
for ($i = 0; $i < 3; $i++) {
    echo (new Node(100 + $i))->name(), "\n";
    echo "after $i\n";
}

// 3. A call's result, chained; the object in the middle is released when
// the chain is done with it.
for ($i = 0; $i < 3; $i++) {
    $v = 1;
    echo $a->next()->bump($v)->name(), " $v\n";
    echo $a->me()->me()->add(1)->n, "\n";
}

// 4. The object through a reference in a temporary: a function that
// returns by reference, a static, an array's element.
class Holder {
    public $node;
    static $kept;
    function __construct() { $this->node = new Node(500); }
    function &ref() { return $this->node; }
}
$h = new Holder;
Holder::$kept = new Node(600);
$list = [new Node(700)];
$alias = &$list[0];
for ($i = 0; $i < 3; $i++) {
    $v = 1;
    echo $h->ref()->bump($v)->n, ' ', Holder::$kept->bump($v)->n, ' ', $list[0]->bump($v)->n, " $v\n";
}

// 5. An undefined variable sent to a method of a temporary: the warning,
// then the call, the object still there.
function warn_once($node) {
    return $node->me()->add($nothing)->n;
}
echo warn_once($a), "\n";
echo warn_once($a), "\n";

// 6. The argument is the last holder of another object: its destructor
// runs where PHP runs it.
function last_holder() {
    $gone = new Node(900);
    $r = (new Node(800))->add(1)->name();
    $gone = null;
    return $r;
}
echo last_holder(), "\n";
echo last_holder(), "\n";
echo "end\n";

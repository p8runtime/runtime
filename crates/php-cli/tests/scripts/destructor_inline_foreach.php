<?php
// foreach's loop variable held the last reference to an object: the
// destructor runs when the next element replaces it, before the body.
class D { public $n; function __construct($n) { $this->n = $n; } function __destruct() { echo "D{$this->n}\n"; } }
function f() {
    $v = new D(0);
    foreach ([1, 2] as $v) { echo "body $v\n"; }
    $v = new D(9);
    foreach ([new D(1), new D(2)] as $v) { echo "body {$v->n}\n"; }
    echo "after\n";
}
f();
echo "end\n";

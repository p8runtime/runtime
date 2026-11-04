<?php
// A destructor that throws while a builtin or an instruction frees its
// object: the exception reaches the try around that code (php-src
// gh19926), with several objects, one, and from a method.
class ThrowingDestructor {
    function __construct(public $n = 0) {}
    function __destruct() { echo "destruct {$this->n}\n"; throw new Exception("from {$this->n}"); }
}
function show($e) {
    $chain = [];
    for ($x = $e; $x; $x = $x->getPrevious()) { $chain[] = $x->getMessage(); }
    echo "caught: ", implode(' <- ', $chain), "\n";
}

$arr = [new ThrowingDestructor(1), new ThrowingDestructor(2)];
try { array_splice($arr, 0, 2); echo "not here\n"; }
catch (Exception $e) { show($e); }
echo "after splice of two\n";

$one = [new ThrowingDestructor(3)];
try { array_splice($one, 0, 1); echo "not here\n"; }
catch (Exception $e) { show($e); }
echo "after splice of one\n";

class Owner {
    public $items;
    function __construct() { $this->items = [new ThrowingDestructor(4), new ThrowingDestructor(5)]; }
    function clear() { $this->items = null; echo "not here\n"; }
}
$o = new Owner;
try { $o->clear(); }
catch (Exception $e) { show($e); }
echo "after method\n";

try { $x = new ThrowingDestructor(6); $x = null; echo "not here\n"; }
catch (Exception $e) { show($e); }
echo "after assign\n";

$three = [new ThrowingDestructor(7), new ThrowingDestructor(8), new ThrowingDestructor(9)];
try { $three = []; echo "not here\n"; }
catch (Exception $e) { show($e); }
echo "end\n";

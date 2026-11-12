<?php
// A generator or a fiber suspended between a method call's start (its
// receiver taken, its method found) and the call itself: the call goes on
// with the same receiver and method when it is resumed, and lets go of the
// receiver when it never is.
class A {
    public $name;
    function __construct($name) { $this->name = $name; echo "[new {$this->name}] "; }
    function m(&$x, $y) { $x .= "+m"; return "A::m on {$this->name}($x, $y)"; }
    function __destruct() { echo "[{$this->name} gone] "; }
}
class B extends A {
    function m(&$x, $y) { return "B::m on {$this->name}"; }
}

function gen() {
    $o = new A("in generator");
    $x = "x";
    $r = $o->m($x, yield "first");
    echo $r, " x=$x\n";
    // The receiver replaced while the generator is suspended.
    $x = "x";
    $r = $o->m($x, [yield "second", $o = new B("replacement")][0]);
    echo $r, " x=$x ", get_class($o), "\n";
    // A receiver that is only a temporary, across two suspensions.
    $r = (new A("temporary"))->m($x, (yield "third") . (yield "fourth"));
    echo $r, "\n";
    return "done";
}
$g = gen();
echo $g->current(), "\n";
echo $g->send("sent 1"), "\n";
echo $g->send("sent 2"), "\n";
echo $g->send("sent 3"), "\n";
$g->send("sent 4");
echo $g->getReturn(), "\n";
$g = null;
echo "\n";

// A generator dropped while suspended there: the receiver goes with it.
function dropped() {
    $x = "x";
    echo (new A("dropped"))->m($x, yield 1), "\n";
}
$g = dropped();
$g->current();
echo "dropping: ";
$g = null;
echo "dropped\n";

// One thrown into: the receiver goes as the exception leaves the call.
function thrown_into() {
    $x = "x";
    try {
        echo (new A("thrown into"))->m($x, yield 1), "\n";
    } catch (Exception $e) {
        echo "caught ", $e->getMessage(), "\n";
    }
    yield 2;
}
$g = thrown_into();
$g->current();
echo $g->throw(new Exception("in the argument")), "\n";
$g = null;

// A nested call suspended inside an argument, and yield from there.
function inner() { yield "inner"; return "inner returned"; }
function outer() {
    $o = new A("outer");
    $x = "x";
    echo $o->m($x, $o->m($x, yield "nested")), " x=$x\n";
    echo $o->m($x, $o->m($x, yield from inner())), " x=$x\n";
}
$g = outer();
echo $g->current(), "\n";
echo $g->send("value"), "\n";
$g->next();
$g = null;
echo "|\n";

// A fiber suspended in an argument; resumed, thrown into, abandoned.
$fiber = new Fiber(function () {
    $o = new A("in fiber");
    $x = "x";
    $r = $o->m($x, Fiber::suspend("suspended"));
    echo $r, " x=$x\n";
    $o = null;
    try {
        echo (new A("fiber temporary"))->m($x, Fiber::suspend("again")), "\n";
    } catch (Exception $e) {
        echo "caught ", $e->getMessage(), "\n";
    }
    echo (new A("abandoned"))->m($x, Fiber::suspend("last")), "\n";
});
echo $fiber->start(), "\n";
echo $fiber->resume("resumed with"), "\n";
echo $fiber->throw(new Exception("thrown into the fiber")), "\n";
echo "abandoning: ";
$fiber = null;
gc_collect_cycles();
echo "abandoned\n";

// A method that suspends its caller's fiber, called many times at one site.
class Yielder {
    function step(&$n) { $n++; return Fiber::suspend($n); }
}
$fiber = new Fiber(function () {
    $y = new Yielder; $n = 0; $sum = 0;
    for ($i = 0; $i < 3; $i++) { $sum += $y->step($n); }
    return "$n $sum";
});
$v = $fiber->start();
while (!$fiber->isTerminated()) { $v = $fiber->resume($v * 10); }
echo $fiber->getReturn(), "\n";

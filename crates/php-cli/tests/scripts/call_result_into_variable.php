<?php
// `$v = f(..)`: the result is stored in the variable when the callee
// returns. What PHP shows of the moment it happens: a destructor of the
// callee's locals runs before the assignment (it sees the old value, and
// if it throws nothing is assigned); the old value's destructor runs after
// it (it sees the new one). Each case runs twice: a call site's first call
// finds the function, and the calls after it are the ones that store the
// result themselves.

class D {
    function __construct(public $n) {}
    function __destruct() {
        echo "D{$this->n} g=", json_encode($GLOBALS['g'] ?? null), "\n";
    }
}
class Thrower {
    function __destruct() { throw new Exception('from the destructor'); }
}
class Setter {
    function __destruct() { $GLOBALS['g'] = 'set by the destructor'; }
}

function plain() { return 5; }
function with_local() { $d = new D(1); return 6; }
function with_arg($d) { return 7; }
function throws_at_exit() { $t = new Thrower; return 8; }
function sets_at_exit() { $s = new Setter; return 9; }
function throws() { throw new Exception('thrown'); }
function nothing() {}
function &by_ref() { static $x = 3; return $x; }
function same($x) { return $x; }
function gen() { yield 1; yield 2; return 3; }
function line() { return debug_backtrace()[0]['line']; }
function defaults($a, $b = 'b', $c = 'c') { return "$a$b$c"; }
function extra($a) { return $a . ':' . implode(',', func_get_args()); }
function fact($n) { if ($n < 2) { return 1; } $r = fact($n - 1); return $n * $r; }

// The callee's local dies before the assignment: its destructor sees the
// old value of the variable (a global bound by reference here).
function local_first() {
    global $g;
    $g = 'old';
    $g = with_local();
    echo "g=$g\n";
    $g = 'old';
    $g = with_arg(new D(2));
    echo "g=$g\n";
}
local_first();
local_first();

// The old value dies after the assignment: its destructor sees the new one.
function old_after() {
    global $g;
    $g = new D(3);
    $g = plain();
    echo "g=$g\n";
    // Both: the callee's local first, with the old value still there.
    $g = new D(4);
    $g = with_local();
    echo "g=$g\n";
}
old_after();
old_after();

// A destructor that throws as the callee returns: nothing is assigned.
function throwing_local() {
    $v = 'old';
    try {
        $v = throws_at_exit();
    } catch (Exception $e) {
        echo $e->getMessage(), ": v=$v\n";
    }
    try {
        $v = throws();
    } catch (Exception $e) {
        echo $e->getMessage(), ": v=$v\n";
    }
    // The old value's destructor throws: the assignment happened.
    $v = new Thrower;
    try {
        $v = plain();
    } catch (Exception $e) {
        echo $e->getMessage(), ": v=$v\n";
    }
}
throwing_local();
throwing_local();

// A destructor that writes the variable as the callee returns: the
// assignment comes after it.
function written_at_exit() {
    global $g;
    $g = 'old';
    $g = sets_at_exit();
    echo "g=$g\n";
}
written_at_exit();
written_at_exit();

// Through a reference the variable holds: a reference to a local, a static
// variable, a property, an array element.
function references() {
    $a = 1;
    $v = &$a;
    $v = plain();
    echo "a=$a\n";
    static $s = 0;
    $s = plain();
    $o = new stdClass;
    $o->p = 1;
    $p = &$o->p;
    $p = with_local();
    echo "p={$o->p}\n";
    $arr = [1, 2];
    $e = &$arr[1];
    $e = plain();
    echo json_encode($arr), "\n";
    return $s;
}
echo references(), "\n";
echo references(), "\n";

// A function that returns by reference, called for its value.
function by_value() {
    $v = by_ref();
    $v++;
    $w = by_ref();
    echo "v=$v w=$w\n";
}
by_value();
by_value();

// Nothing returned; an array (the caller's copy is its own); the variable
// itself passed and returned; a generator; recursion; a branch that joins.
function values($c) {
    $v = 1;
    $v = nothing();
    var_dump($v);
    $a = [1, 2];
    $b = same($a);
    $b[] = 3;
    $a = same($a);
    $a[] = 4;
    echo json_encode([$a, $b]), "\n";
    $g = gen();
    foreach ($g as $x) { echo $x; }
    echo $g->getReturn(), "\n";
    $f = fact(10);
    echo "$f\n";
    $t = $c ? plain() : with_local();
    echo "t=$t\n";
    $t = $c ? with_local() : plain();
    echo "t=$t\n";
}
$g = 'main';
values(true);
values(false);
values(true);
values(false);

// The line of the call, seen from the callee.
function lines() {
    $l = line();
    $m = line();
    echo $l - __LINE__, ' ', $m - __LINE__, "\n";
}
lines();
lines();

// Defaults and extra arguments.
function counts() {
    $v = defaults(1);
    $w = defaults(1, 2);
    $x = defaults(1, 2, 3);
    $y = extra(1, 2, 3);
    $z = extra(1);
    echo "$v $w $x $y $z\n";
    try {
        $v = defaults();
    } catch (ArgumentCountError $e) {
        echo get_class($e), " v=$v\n";
    }
}
counts();
counts();

// In a fiber: the callee suspends, and its result is stored on return.
function suspends($n) { $d = new D($n); return Fiber::suspend($n) + 1; }
function in_fiber() {
    $x = 0;
    for ($i = 1; $i <= 2; $i++) {
        $x = suspends($x + 10 * $i);
        echo "x=$x\n";
    }
    $x = suspends(30);
    return $x;
}
for ($round = 0; $round < 2; $round++) {
    $fiber = new Fiber('in_fiber');
    echo $fiber->start(), "\n";
    echo $fiber->resume(41), "\n";
    echo $fiber->resume(51), "\n";
    $fiber->resume(61);
    echo $fiber->getReturn(), "\n";
}

// At the top level, where the variables are the globals.
for ($round = 0; $round < 2; $round++) {
    $g = new D(5);
    $g = plain();
    echo "g=$g\n";
    $g = new D(6);
    $g = with_local();
    echo "g=$g\n";
}

// In a method, and a closure.
class C {
    public $p = 0;
    function run() {
        $v = plain();
        $this->p = $v;
        $f = function () { $d = new D(7); $r = with_local(); return $r; };
        $v = $f();
        return $v + $this->p;
    }
}
$c = new C;
$r = $c->run();
echo "r=$r\n";
$r = $c->run();
echo "r=$r\n";

// A function with more temporaries than the compiler frees one by one (a
// string of eighty parts): one may still hold a dead value, here the
// object echoed, which dies as the call's result lands on it.
class Shown extends D {
    function __toString() { return "shown{$this->n}\n"; }
}
function wide($s) {
    $n = strlen("{$s}a{$s}b{$s}c{$s}d{$s}e{$s}f{$s}g{$s}h{$s}i{$s}j{$s}k{$s}l{$s}m{$s}n{$s}o{$s}p{$s}q{$s}r{$s}s{$s}t{$s}u{$s}v{$s}w{$s}x{$s}y{$s}z{$s}a{$s}b{$s}c{$s}d{$s}e{$s}f{$s}g{$s}h{$s}i{$s}j{$s}k{$s}l{$s}m{$s}n");
    echo new Shown(8);
    $v = plain();
    echo "after the call\n";
    $w = new D(9);
    $w = with_arg(new D(10));
    return "$v $n $w";
}
echo wide('x'), "\n";
echo wide('y'), "\n";
echo wide('z'), "\n";

// A result that is not there to store (a warning first), in a loop.
function not_set() { return $nothing; }
function loops() {
    $v = 'old';
    for ($i = 0; $i < 3; $i++) {
        $v = not_set();
        var_dump($v);
        $v = with_local();
        $v = by_ref();
        $v = nothing();
    }
    $objects = [];
    for ($i = 0; $i < 3; $i++) {
        $o = new D(20 + $i);
        $o = with_arg($o);
        $objects[] = $o;
        $t = 'kept';
        try {
            $t = throws_at_exit();
        } catch (Exception $e) {
            echo $e->getMessage(), ": t=$t\n";
        }
    }
    echo json_encode($objects), "\n";
}
loops();
echo "end\n";

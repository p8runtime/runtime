<?php
// `f($a)`, `f($a, $b)`, `f($a, $b, $c)`, `f('k')` on user functions: the
// arguments are copied in order, each as its own step (an undefined one
// warns where it is, before the call), then the function is entered. Each
// case runs twice: a call site's first call finds the function, and the
// calls after it are the ones made from the copies.

class D {
    function __construct(public $n) {}
    function __destruct() { echo "D{$this->n} "; }
}

function one($a) { return "one(" . json_encode($a) . ")"; }
function two($a, $b) { return "two(" . json_encode($a) . "," . json_encode($b) . ")"; }
function three($a, $b, $c) { return "three(" . json_encode([$a, $b, $c]) . ")"; }
function add($a, $b) { return $a + $b; }
function changes($a, $b) { $a[] = 9; $b = 'changed'; return count($a); }
function by_ref(&$a, $b) { $a .= $b; return strlen($a); }
function opt($a, $b = 'B') { return "$a$b"; }
function gen($a, $b) { yield $a; yield $b; }
function drop($o) { echo "in "; }
function fib($n) { return $n < 2 ? $n : fib($n - 1) + fib($n - 2); }
function thrower($a, $b) { throw new Exception("thrown with $a,$b"); }
function locals($a) { $x = $a + 1; $y = $x + 1; return [$a, $x, $y, isset($z)]; }
function rest($a) { $x = 'x'; return $a . $x . ':' . implode(',', func_get_args()); }
function opts($a, $b = 'B', $c = ['C']) { $x = 'x'; return $a . $b . $c[0] . $x . func_num_args(); }
function needs($a, $b, $c = 'C') { return "$a$b$c"; }

function plain() {
    $a = 1; $b = 'two'; $c = [3];
    echo one($a), ' ', two($a, $b), ' ', three($a, $b, $c), "\n";
    $v = one($a);
    $w = two($b, $c);
    $x = three($c, $b, $a);
    echo "$v $w $x\n";
    echo one('k'), ' ', one(5), ' ', one(null), "\n";
    $k = one('k');
    echo "$k\n";
    // The result goes to a variable that is an argument.
    $s = 0;
    for ($i = 0; $i < 5; $i++) {
        $s = add($s, $i);
    }
    $a = add($a, $a);
    echo "$s $a\n";
    echo json_encode(locals($a)), json_encode(locals($s)), "\n";
}
plain();
plain();

// An undefined argument warns where it is read: the call is still made.
function undefined() {
    $a = 1;
    echo one($u), "\n";
    echo two($a, $u), "\n";
    echo two($u, $a), "\n";
    echo three($a, $u1, $u2), "\n";
    $v = three($u1, $a, $u2);
    echo "$v\n";
}
undefined();
undefined();

// A handler that throws for the warning: the call is not made.
function stopped() {
    $a = 1;
    $v = 'old';
    set_error_handler(function ($no, $msg) { throw new ErrorException($msg); });
    try {
        $v = two($a, $u);
    } catch (ErrorException $e) {
        echo $e->getMessage(), ": v=$v\n";
    }
    try {
        $v = three($u, $a, $a);
    } catch (ErrorException $e) {
        echo $e->getMessage(), ": v=$v\n";
    }
    restore_error_handler();
    $v = two($a, $a);
    echo "$v\n";
}
stopped();
stopped();

// By value: the callee's changes are its own; a variable that holds a
// reference (a global, a static, a reference to another) passes its value.
function values() {
    global $g;
    static $s = 'static';
    $arr = [1, 2];
    $str = 'str';
    $n = changes($arr, $str);
    echo $n, json_encode($arr), $str, "\n";
    $r = &$str;
    echo two($g, $s), ' ', two($r, $g), ' ', three($g, $s, $r), "\n";
    $n = changes($arr, $r);
    echo $n, $str, "\n";
    // A by-reference parameter is not a copy.
    $t = 'a';
    $n = by_ref($t, $str);
    echo "$n $t\n";
}
$g = 'global';
values();
values();

// Fewer arguments than parameters, a generator, an exception.
function others() {
    $a = 'a'; $b = 'b';
    echo opt($a), opt($a, $b), "\n";
    $v = opt($a);
    $w = opt($a, $b);
    echo "$v$w\n";
    // More arguments than parameters (func_get_args() has them), and
    // fewer (the defaults); a required one missing is an error.
    $c = 'c';
    echo rest($a, $b), ' ', rest($a, $b, $c), ' ', opts($a), ' ', opts($a, $b), ' ', opts($a, $b, $c), "\n";
    $v = rest($a, $b, $c);
    $w = opts($b);
    echo "$v $w\n";
    for ($i = 0; $i < 3; $i++) {
        $v = opts($v);
        $w = rest($w, $v);
    }
    echo "$v $w\n";
    try {
        $v = needs($a);
    } catch (ArgumentCountError $e) {
        echo get_class($e), ": v=$v\n";
    }
    echo needs($a, $b), "\n";
    foreach (gen($a, $b) as $x) { echo $x; }
    $it = gen($b, $a);
    foreach ($it as $x) { echo $x; }
    echo "\n";
    try {
        $v = thrower($a, $b);
    } catch (Exception $e) {
        echo $e->getMessage(), ": v=$v\n";
    }
    echo fib(15), "\n";
}
others();
others();

// An object passed: it lives as long as the caller's variable, or the
// call.
function objects() {
    $o = new D(1);
    drop($o);
    echo "after\n";
    drop(new D(2));
    echo "after\n";
    $p = new D(3);
    $v = one($p);
    echo "$v\n";
    $p = one($p);
    echo "$p\n";
}
objects();
echo "\n";
objects();
echo "\n";

// At the top level (the globals), in a method, in a closure.
$a = 10; $b = 20;
for ($round = 0; $round < 2; $round++) {
    $v = add($a, $b);
    echo two($a, $b), " $v\n";
}
class C {
    public $p = 'p';
    function run($a) {
        $b = $this->p;
        $f = function ($x) use ($a) { $r = two($a, $x); return $r; };
        return three($a, $b, $this->p) . ' ' . $f($b) . ' ' . two($this->p, $a);
    }
}
$c = new C;
echo $c->run($a), "\n";
echo $c->run($b), "\n";
echo "end\n";

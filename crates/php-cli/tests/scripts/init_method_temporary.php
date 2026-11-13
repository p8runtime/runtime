<?php
// A receiver that lives only in a temporary (`(new D)->m()`, a call's
// result): when its destructor runs, against everything around the call.
class D {
    public $name;
    function __construct($name) { $this->name = $name; echo "[new $name] "; }
    function m($x = null) { echo "m($x) "; return "result of {$this->name}"; }
    function ref(&$x) { $x .= "+"; echo "ref($x) "; return $this; }
    function me() { echo "me "; return $this; }
    function fail($x = null) { echo "fail "; throw new Exception("from {$this->name}"); }
    function keep() { echo "keep "; return function () { return $this->name; }; }
    function __destruct() { echo "[{$this->name} gone] "; }
}
class Quiet { function m($x = null) { return "quiet"; } }
function make($name) { return new D($name); }
function eff($v) { echo "eff($v) "; return $v; }
function thrower() { echo "thrower "; throw new Exception("from the argument"); }

function run() {
    // No argument; the result unused, used, assigned.
    (new D("a"))->m(); echo "| after a\n";
    echo (new D("b"))->m(), " | after b\n";
    $r = (new D("c"))->m(); echo "| after c: $r\n";
    // Arguments: a literal, a variable, a call.
    (new D("d"))->m(1); echo "| after d\n";
    $v = "v";
    (new D("e"))->m($v); echo "| after e\n";
    (new D("f"))->m(eff("arg")); echo "| after f\n";
    (new D("g"))->ref($v); echo "| after g $v\n";
    // A call's result as the receiver.
    make("h")->m(eff("arg")); echo "| after h\n";
    // A chain: each link's receiver is the one before.
    (new D("i"))->me()->me()->m(); echo "| after i\n";
    make("j")->ref($v)->ref($v)->m(eff("arg")); echo "| after j $v\n";
    // Two temporaries in one expression.
    echo (new D("k"))->m(), " ", (new D("l"))->m(), " | after k l\n";
    echo (new D("m"))->m((new D("n"))->m()), " | after m n\n";
    // The method throws; an argument throws.
    try { (new D("o"))->fail(); } catch (Exception $e) { echo "| caught ", $e->getMessage(), "\n"; }
    try { (new D("p"))->fail(eff("arg")); } catch (Exception $e) { echo "| caught ", $e->getMessage(), "\n"; }
    try { (new D("q"))->m(thrower()); } catch (Exception $e) { echo "| caught ", $e->getMessage(), "\n"; }
    // (Two receivers held when an argument throws: both are released
    // before the catch; in which order is not printed, p8 releases any two
    // temporaries lowest first where PHP releases the inner call's first.)
    try { (new D("r"))->m(eff(1), (new Quiet)->m(thrower())); } catch (Exception $e) { echo "| caught ", $e->getMessage(), "\n"; }
    try { (new D("t"))->nope(eff("never")); } catch (Error $e) { echo "| caught ", $e->getMessage(), "\n"; }
    // The method's result keeps the object.
    $f = (new D("u"))->keep(); echo "| after u: ", $f(), " ";
    $f = null; echo "| closure dropped\n";
    // In a loop: one site, a new temporary each time.
    for ($i = 0; $i < 3; $i++) { (new D("loop $i"))->m($i); }
    echo "| after the loop\n";
    // A variable's object, the variable cleared by an argument: the call
    // holds the last count.
    $o = new D("v");
    $o->m($o = null); echo "| after v\n";
    $o = new D("w");
    $o->m(eff("arg"), $o = null) . print("| call w done ");
    echo "| after w\n";
    // Nullsafe on a temporary.
    (new D("x"))?->m(eff("arg")); echo "| after x\n";
    // A static method and __call on a temporary.
    echo "end of run\n";
}
run();

class Magic {
    public $name;
    function __construct($name) { $this->name = $name; echo "[new $name] "; }
    function __call($n, $a) { echo "__call $n "; return 1; }
    static function st($x = null) { echo "st "; return 2; }
    function __destruct() { echo "[{$this->name} gone] "; }
}
(new Magic("magic 1"))->anything(eff("arg")); echo "| after magic 1\n";
$kept = new Magic("magic 2");
$kept->st(eff("arg")); echo "| after magic 2\n";
$kept = null; echo "| magic 2 dropped\n";
// A builtin's method on a temporary.
echo (new ArrayObject([1, 2, 3]))->count(), " ", (new DateTime("2026-01-02", new DateTimeZone("UTC")))->format(eff("Y")), "\n";

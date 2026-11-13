<?php
// A lazy object as a method call's receiver: its initializer runs
// arbitrary code (here it replaces the variable the call was made
// through) when the method first touches a property, long after the
// method was found.
class Real {
    public $value = "unset";
    public $log = "";
    function __construct($value = "constructed") { $this->value = $value; }
    function read(&$x, $y = null) { $x .= "+read"; return "Real::read " . $this->value . " (" . get_class($this) . ")"; }
    function plain() { return "Real::plain"; }
}
class Other {
    function read($x, $y = null) { return "Other::read"; }
    function plain() { return "Other::plain"; }
}
function eff($v) { echo "eff($v) "; return $v; }

function ghost() {
    $rc = new ReflectionClass(Real::class);
    $o = null;
    $o = $rc->newLazyGhost(function (Real $obj) use (&$o) {
        echo "(initializer) ";
        $o = new Other;
        $obj->__construct("from the initializer");
    });
    // No property touched: not initialized.
    echo $o->plain(), " ", $rc->isUninitializedLazyObject($o) ? "lazy" : "initialized", "\n";
    $x = "x";
    $kept = $o;
    echo $o->read($x, eff("arg")), " x=$x\n";
    echo get_class($o), " ", $rc->isUninitializedLazyObject($kept) ? "lazy" : "initialized", "\n";
    // The site again, now on the replacement, then on the initialized ghost.
    foreach ([$o, $kept, $o] as $each) {
        $x = "x";
        echo $each->read($x, eff("arg")), " x=$x\n";
    }
}
ghost();

function proxy() {
    $rc = new ReflectionClass(Real::class);
    $o = null;
    $o = $rc->newLazyProxy(function (Real $proxy) use (&$o) {
        echo "(factory) ";
        $o = new Other;
        return new Real("the real instance");
    });
    $x = "x";
    $kept = $o;
    echo $o->read($x, eff("arg")), " x=$x\n";
    echo get_class($o), " ", get_class($kept), "\n";
    $x = "x";
    echo $kept->read($x), " x=$x\n";
}
proxy();

// An initializer that throws: the call's error is the initializer's, and
// the object stays lazy for the next call.
function failing() {
    $rc = new ReflectionClass(Real::class);
    $tries = 0;
    $o = $rc->newLazyGhost(function (Real $obj) use (&$tries) {
        if (++$tries < 2) { throw new Exception("initializer failed"); }
        $obj->value = "second try";
    });
    for ($i = 0; $i < 2; $i++) {
        $x = "x";
        try { echo $o->read($x, eff("arg")), " x=$x\n"; } catch (Exception $e) { echo "caught ", $e->getMessage(), " x=$x\n"; }
    }
}
failing();

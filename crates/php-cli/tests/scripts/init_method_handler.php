<?php
// What an "Undefined variable" warning's handler does between the method
// being found and its frame existing: it throws, it replaces or unsets
// the receiver's variable. (A handler that suspends the fiber is not here:
// p8 cannot suspend a fiber from an error handler, with or without a
// method call.)
class A {
    public $name;
    function __construct($name = "a") { $this->name = $name; }
    function m($x, $y = null) { return "A::m on {$this->name}(" . var_export($x, true) . ")"; }
    function __destruct() { echo "[{$this->name} gone] "; }
}
class B extends A {
    function m($x, $y = null) { return "B::m on {$this->name}(" . var_export($x, true) . ")"; }
}

// It throws: the method never runs; a receiver held only by the call is
// released as the exception leaves the call.
set_error_handler(function ($no, $message) { throw new Exception($message); });
function throws() {
    $o = new A("kept");
    try { echo $o->m($undefined), "\n"; } catch (Exception $e) { echo "caught: ", $e->getMessage(), "\n"; }
    try { echo (new A("temporary"))->m($undefined), "\n"; } catch (Exception $e) { echo "caught: ", $e->getMessage(), "\n"; }
    try { echo (new A("temporary 2"))->m(1, $undefined), "\n"; } catch (Exception $e) { echo "caught: ", $e->getMessage(), "\n"; }
    echo "end of throws\n";
}
throws();
echo "|\n";

// It replaces the receiver's variable: the object the call started on is
// the one called, and is released when the call is done.
$o = null;
set_error_handler(function ($no, $message) {
    echo "(handler: $message) ";
    $GLOBALS['o'] = new B("replacement");
    return true;
});
$o = new A("first");
echo $o->m($undefined), "\n";
echo get_class($o), " ", $o->name, "\n";

// It unsets it.
set_error_handler(function ($no, $message) {
    echo "(handler: $message) ";
    unset($GLOBALS['o']);
    return true;
});
$o = new A("second");
echo $o->m($undefined), "\n";
echo isset($o) ? "still set" : "unset", "\n";

// The same through a reference to the receiver's variable, in a function.
function replaced() {
    $o = new A("third");
    $ref = &$o;
    set_error_handler(function ($no, $message) use (&$ref) {
        echo "(handler: $message) ";
        $ref = new B("replacement 3");
        return true;
    });
    echo $o->m($undefined), "\n";
    echo get_class($o), " ", $o->name, "\n";
    $ref = null;
    echo "|\n";
}
replaced();
echo "end\n";
restore_error_handler();

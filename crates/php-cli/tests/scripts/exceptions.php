<?php
function f($x) {
    if ($x > 2) throw new InvalidArgumentException("too big: $x", 42);
    return $x * 2;
}
try {
    echo f(1), "\n";
    echo f(5), "\n";
    echo "not reached\n";
} catch (InvalidArgumentException $e) {
    echo get_class($e), " ", $e->getMessage(), " ", $e->getCode(), " ", $e->getLine(), "\n";
    echo $e->getTraceAsString(), "\n";
} finally {
    echo "finally\n";
}
try { throw new RuntimeException("rt"); } catch (LogicException $e) { echo "logic\n"; } catch (RuntimeException | TypeError $e) { echo "multi ", $e->getMessage(), "\n"; }
function g() {
    try { return "try"; } finally { echo "g-finally\n"; }
}
echo g(), "\n";
function h() {
    try { throw new Exception("x"); } catch (Exception $e) { return "catch"; } finally { echo "h-finally\n"; }
}
echo h(), "\n";
function k() { try { return 1; } finally { return 2; } }
echo k(), "\n";
for ($i = 0; $i < 3; $i++) {
    try { if ($i == 1) continue; echo "i=$i\n"; } finally { echo "f$i\n"; }
}
try { strlen(); } catch (ArgumentCountError $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { intdiv(1, 0); } catch (DivisionByZeroError $e) { echo $e->getMessage(), "\n"; }
try { $x = 1 % 0; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
try { undefined_fn(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
$prev = new Exception("inner");
$e = new LogicException("outer", 3, $prev);
var_dump($e->getPrevious() === $prev, $e instanceof Throwable, $e instanceof Exception, $e instanceof Error);
class MyEx extends Exception {
    public function __construct($m) { parent::__construct("my: " . $m, 7); }
}
try { throw new MyEx("boom"); } catch (Exception $e) { echo $e->getMessage(), $e->getCode(), "\n"; }
function nested() {
    try {
        try { throw new Exception("deep"); }
        finally { echo "inner finally\n"; }
    } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; throw new RuntimeException("again", 0, $e); }
}
try { nested(); } catch (RuntimeException $e) { echo $e->getMessage(), " <- ", $e->getPrevious()->getMessage(), "\n"; }
try { throw new Exception(); } catch (Exception) { echo "no var\n"; }
echo "done\n";

<?php
// A destructor that throws, run by an inline write inside a try: caught
// by that try, not by whatever surrounds the next slow instruction.
class D { function __destruct() { throw new Exception("boom"); } }
function f($a) {
    try { $x = new D; $x = $a + 1; } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; }
    echo "after\n";
}
f(1);

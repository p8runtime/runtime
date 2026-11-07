<?php
// An object's last holder overwritten by an inline arithmetic result: the
// destructor runs right there, before the next statement (#321 review).
class D { function __destruct() { echo "D\n"; } }
function f($a) { $x = new D; $x = $a + 1; echo "after\n"; return $x; }
echo f(1), "\n";

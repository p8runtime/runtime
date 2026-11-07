<?php
// The last holder overwritten by a concatenation.
class D { function __destruct() { echo "D\n"; } }
function f($s, $t) { $x = new D; $x = $s . $t; echo "after ", $x, "\n"; }
f("a", "b");

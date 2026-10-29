<?php
// A callback whose parameter takes a reference, called with a value:
// PHP warns and calls it with the value.
function bump(&$n) { $n++; return $n; }
var_dump(call_user_func('bump', 1));
var_dump(array_map('bump', [1, 2]));
$f = function (&$s) { $s .= '!'; return $s; };
var_dump(call_user_func_array($f, ['x']));
class K { static function m(&$a, $b) { return $b; } }
var_dump(call_user_func('K::m', 1, 2));

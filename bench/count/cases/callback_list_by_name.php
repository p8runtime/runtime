<?php
// One call_user_func_array of a function named by a string, as WP_Hook::apply_filters runs a filter.
$n = (int) ($argv[1] ?? 1000);
function add($v, $x) { return $v + $x; }
function run($n) { $s = 0; $cb = 'add'; for ($i = 0; $i < $n; $i++) { $s = call_user_func_array($cb, [$s, $i]); } return $s; }
echo run($n), "\n";

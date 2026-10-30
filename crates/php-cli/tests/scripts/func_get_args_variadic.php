<?php
// func_get_args() / func_get_arg() / func_num_args() count a variadic
// parameter's arguments one by one (MediaWiki's BagOStuff::makeGlobalKey).
function k($group, ...$rest) { var_dump(func_get_args(), func_num_args()); try { var_dump(func_get_arg(2)); } catch (ValueError $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; } }
k('g', 'a', 'b');
k('g');
class B { function m($a, ...$r) { return func_get_args(); } static function s(...$all) { return func_get_args(); } }
var_dump((new B)->m(1, 2, 3), B::s('x', 'y'));
function spread($a, ...$rest) { var_dump(func_get_args()); }
spread(1, ...[2, 3]);
function byref(&$a, ...$r) { $a = 5; var_dump(func_get_args()); }
$z = 1; byref($z, 9);

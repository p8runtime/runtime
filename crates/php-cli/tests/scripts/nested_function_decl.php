<?php
var_dump(function_exists('is_countable'));
if (!function_exists('is_countable')) { function is_countable($v) { return 1; } }
if (!function_exists('my_f')) { function my_f() { return 'my_f'; } }
echo my_f(), "\n";
var_dump(function_exists('later'));
function outer() { function inner() { return 'inner'; } return 'outer'; }
var_dump(function_exists('inner'));
echo outer(), " ", inner(), "\n";
echo top(), "\n";
function top() { return 'top hoisted'; }
if (true) { function cond(&$x) { $x++; } }
$n = 1; cond($n); echo $n, "\n";
function make() { if (!function_exists('made')) { function made() { return 'made once'; } } }
make(); make(); echo made(), "\n";
var_dump(function_exists('never'));
if (false) { function never() {} }
var_dump(function_exists('never'));

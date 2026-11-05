<?php
// get_error_handler() / get_exception_handler() (PHP 8.5): the handler in
// effect, null when there is none, following set_*/restore_* as a stack.
function eh1($n, $s) { return true; }
function eh2($n, $s) { return true; }
function xh1($e) {}
class H { function err($n, $s) { return true; } static function exc($e) {} }

var_dump(get_error_handler(), get_exception_handler());
set_error_handler('eh1');
set_error_handler('eh2', E_WARNING);
var_dump(get_error_handler());
restore_error_handler();
var_dump(get_error_handler());
$h = new H;
set_error_handler([$h, 'err']);
var_dump(get_error_handler()[1], get_error_handler()[0] === $h);
set_error_handler(null);
var_dump(get_error_handler());
restore_error_handler();
restore_error_handler();
restore_error_handler();
var_dump(get_error_handler());
restore_error_handler();
var_dump(get_error_handler());

$c = function ($e) {};
set_exception_handler('xh1');
set_exception_handler($c);
var_dump(get_exception_handler() === $c);
restore_exception_handler();
var_dump(get_exception_handler());
set_exception_handler(['H', 'exc']);
var_dump(get_exception_handler());
restore_exception_handler();
restore_exception_handler();
var_dump(get_exception_handler());

<?php
echo $undef;
$a = []; echo $a['k'], $a[3];
echo "x" . [1], "\n";
trigger_error("custom notice");
trigger_error("custom warning", E_USER_WARNING);
@trigger_error("silenced", E_USER_WARNING);
var_dump(error_get_last()['message']);
$prev = set_error_handler(function ($no, $str, $file, $line) { echo "handler: $no $str line $line\n"; return true; });
var_dump($prev);
echo $undef2;
trigger_error("to handler", E_USER_DEPRECATED);
set_error_handler(function ($no, $str) { throw new ErrorException($str, 0, $no); });
try { echo $undef3; } catch (ErrorException $e) { echo "caught: ", $e->getMessage(), " severity ", $e->getSeverity(), "\n"; }
restore_error_handler(); restore_error_handler();
var_dump(ini_get('display_errors'), ini_set('display_errors', '0'), ini_get('display_errors'), ini_get('nope'));
echo $hidden;
ini_set('display_errors', '1');
set_exception_handler(function ($e) { echo "exception handler: ", $e->getMessage(), "\n"; });
ob_start(); echo "in buffer\n";
throw new RuntimeException("boom");

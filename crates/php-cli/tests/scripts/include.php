<?php
$x = 4;
$r = include 'include_helper.inc';
var_dump($r, $x, $shared, helper_fn(1), HelperClass::V);
var_dump(include_once 'include_once.inc');
var_dump(include_once 'include_once.inc');
var_dump(require __DIR__ . '/include_noreturn.inc');
function inside() {
    $x = 2;
    include 'include_noreturn.inc';
    return $x;
}
echo inside(), "\n";
$r = @include 'missing.inc';
var_dump($r);
$r = include 'missing.inc';
try { require 'missing.inc'; } catch (Error $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
var_dump(include_once __FILE__);

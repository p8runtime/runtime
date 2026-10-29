<?php
// $_SERVER, $_GET… in a file included from a function or a method, and in
// eval()'d code in a function, are the global arrays (PHPUnit includes
// WordPress's files from inside its loader's methods).
function f() { $where = 'function'; include __DIR__ . '/superglobals_in_includes/part.inc'; }
class L { static function load() { $where = 'method'; include __DIR__ . '/superglobals_in_includes/part.inc'; } }
f();
L::load();
$where = 'top level';
include __DIR__ . '/superglobals_in_includes/part.inc';
function g() { eval('echo "eval: ", is_array($_SERVER) ? "array" : "not an array", "\n"; $_GET["seen"][] = "eval";'); }
g();
echo implode(', ', $_GET['seen']), "\n";

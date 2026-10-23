<?php
function f($a, $b = 'B', $c = 'C', ...$rest) { return "$a-$b-$c-" . count($rest); }
echo f(1, c: 3), "\n";
echo f(a: 1), "\n";
echo f(c: 'x', a: 'y'), "\n";
echo f(...['a' => 1, 'c' => 2]), "\n";
echo f(1, ...['c' => 9]), "\n";
try { f(b: 2); } catch (ArgumentCountError $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
try { f(1, a: 2); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { f(1, zz: 2); } catch (Error $e) { echo $e->getMessage(), "\n"; }
class P { function __construct(public $x = 0, public $y = 0, public $z = 0) {} function m($p, $q = 'q') { return "$p/$q"; } static function s($u, $v = 'v') { return "$u|$v"; } }
var_dump(new P(z: 3, x: 1));
$p = new P(y: 5);
echo $p->m(q: 'Q', p: 'P'), "\n";
echo P::s(v: 'V', u: 'U'), "\n";
echo str_pad('x', 5, pad_type: STR_PAD_LEFT), "|\n";
echo htmlspecialchars('<a href="x">', double_encode: false), "\n";
var_dump(json_encode(['a' => 1], flags: JSON_PRETTY_PRINT));
var_dump(array_slice([1, 2, 3, 4], offset: 1, preserve_keys: true));
echo implode(separator: ',', array: [1, 2]), "\n";
$fn = fn($x, $y = 2) => $x * $y;
echo $fn(y: 10, x: 3), "\n";

var_dump(in_array(needle: '1', haystack: [1, 2], strict: true));
function g(...$all) { var_dump($all); }
g(1, 2, x: 3, y: 4);

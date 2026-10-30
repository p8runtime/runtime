<?php
function f($a, &$out = null) { $out = "set"; return 1; }
f(1, out: $x); var_dump($x);
class T { static function p($a, ?string &$parsedName = null) { $parsedName = "n"; } function m(&...$r) {} }
T::p(1, parsedName: $y); var_dump($y);
$args = [1]; f(...$args, out: $z); var_dump($z);
preg_match('/(b)/', 'ab', flags: PREG_OFFSET_CAPTURE, matches: $m); var_dump($m);
function v($a, $b = 0) { $b = 9; }
$keep = 1; v(1, b: $keep); var_dump($keep);
$arr = ['k' => 0]; f(1, out: $arr['k']); var_dump($arr);
$o = new stdClass; f(1, out: $o->p); var_dump($o->p);
try { f(1, nope: $q); } catch (Error $e) { echo $e->getMessage(), "\n"; }

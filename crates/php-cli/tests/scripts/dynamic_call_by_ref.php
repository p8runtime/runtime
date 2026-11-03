<?php
function fr(&$x) { $x = 'f'; }
$c = function (&$x) { $x = 'c'; }; $c($y); var_dump($y);
$n = 'fr'; $n($z); var_dump($z);
class R { static function ap(&$x) { $x = 'm'; } function im(&$x) { $x = 'i'; } }
$m = [new R, 'im']; $m($w); var_dump($w);
$m = [R::class, 'ap']; $m($v); var_dump($v);
class R2 { static function ap($v, array &$args) { $args['config']['k'] = $v; } }
$a = ['fn' => [R2::class, 'ap']];
$args = ['config' => []];
$a['fn'](1, $args); var_dump($args);
$f = [R2::class, 'ap']; $args2 = []; $f(2, $args2); var_dump($args2);
$s = 'R2::ap'; $args3 = []; $s(3, $args3); var_dump($args3);
// By value stays by value; a by-ref element and property are bound.
$byval = function ($x) { $x = 'changed'; }; $keep = 'same'; $byval($keep); var_dump($keep);
$arr = ['k' => 0]; $c($arr['k']); var_dump($arr);
$o = new stdClass; $c($o->p); var_dump($o->p);
$sorter = 'sort'; $list = [3, 1, 2]; $sorter($list); var_dump($list);

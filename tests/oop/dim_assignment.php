<?php
// $base[...] = v writes back through every kind of base and evaluates to v.
$a = []; var_dump($a['x'] = 5); echo json_encode($a), "\n";
$u['k'][] = 1; echo json_encode($u), "\n";
$o = new stdClass; $o->p = []; var_dump($o->p['y'] = 6); print_r($o);
class S { static $s = []; public $q = []; static function t() { self::$s['a'] = 1; static::$s['b'][] = 2; return self::$s; } }
var_dump(S::$s = [1]); echo json_encode(S::t()), "\n";
$o2 = new S; $o2->{'q'}['z'] = 3; print_r($o2);
$n = 'a'; $o3 = new stdClass; $o3->$n = 1; $o3->{'b' . 'c'} = 2; $o3->$n += 5; $o3->arr = [];
$k = 'arr'; $o3->$k['x'] = 1; $o3->{$k}[] = 2; print_r($o3);
$x = ['a' => ['b' => 1]]; $x['a']['c'] = 2; echo json_encode($x), "\n";

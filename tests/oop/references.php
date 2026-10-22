<?php
// References into array elements, properties and static properties.
$a = ['x' => ['y' => 1]]; $r = &$a['x']['y']; $r = 9; echo $a['x']['y'], "\n";
$b = []; $r2 = &$b['new']; $r2 = 3; echo json_encode($b), "\n";
$o = new stdClass; $o->p = ['q' => 1]; $r3 = &$o->p['q']; $r3 = 7; echo $o->p['q'], "\n";
$r4 = &$o->z; $r4 = 5; echo $o->z, "\n";
class C { static $c = ['p' => ['m' => null]]; static $n = 0; }
$r5 = &C::$c['p']; $r5['m'] = 5; var_dump(C::$c['p']['m']);
$r6 = &C::$n; $r6++; echo C::$n, "\n";
function &getRef(array &$arr) { return $arr[0]; } $z = [1]; $rr = &getRef($z); $rr = 2; echo $z[0], "\n";
$list = [1, 2, 3]; foreach ($list as &$v) { $v *= 2; } unset($v); echo implode(',', $list), "\n";
$c1 = ['k' => 1]; $c2 = $c1; $rk = &$c1['k']; $rk = 99; echo $c1['k'], ' ', $c2['k'], "\n";
$app = [1]; $ra = &$app[]; $ra = 'new'; echo json_encode($app), "\n";
class D { static $list = []; static function add($v) { $slot = &self::$list[]; $slot = $v; } } D::add(1); D::add(2); echo json_encode(D::$list), "\n";
$x1 = 1; C::$n = &$x1; $x1 = 42; echo C::$n, "\n";
$y1 = 1; $o5 = new stdClass; $o5->p = &$y1; $y1 = 8; echo $o5->p, "\n";

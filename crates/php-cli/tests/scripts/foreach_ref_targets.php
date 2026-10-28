<?php
// foreach by reference writes through to a property, a static property,
// and elements of variables and properties.
class P { public $it = [[1], [2]]; public $flat = [1, 2]; public static $list = [1, 2]; }
$p = new P;
foreach ($p->it as $i => &$x) { $x = ['new', $i]; } unset($x);
foreach ($p->flat as &$y) { $y *= 10; } unset($y);
foreach (P::$list as &$s) { $s = -$s; } unset($s);
echo json_encode($p->it), json_encode($p->flat), json_encode(P::$list), "\n";
$arr = ['k' => [1, 2]];
foreach ($arr['k'] as &$z) { $z++; } unset($z);
$o = new stdClass; $o->a = ['x' => [1]];
foreach ($o->a['x'] as &$w) { $w = 9; } unset($w);
echo json_encode($arr), json_encode($o), "\n";
class Q { public $it = [[1, 10, 101]]; function go() { foreach ($this->it as $i => &$iteration) { $c = current($iteration); $iteration = [1, 10, 200]; while (current($iteration) < $c) { if (false === next($iteration)) { break; } } } unset($iteration); return [current($this->it[0]), next($this->it[0])]; } }
$q = new Q; next($q->it[0]); echo json_encode($q->go()), "\n";
function f(int $x) {}
try { f('a'); } catch (TypeError $e) { echo explode("\n", str_replace(__FILE__, 'F', (string) $e))[0], "\n"; }

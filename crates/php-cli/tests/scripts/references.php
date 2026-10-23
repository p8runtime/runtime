<?php
class O { public $p = 1; public $arr = [1, 2]; public static $s = 's'; }
$o = new O;
$x = &$o->p; $x = 10; var_dump($o->p);
$o->p = 20; var_dump($x);
$y = 5; $o->q2 = 0; $o->p = &$y; $y = 6; var_dump($o->p);
$o->self = &$o; var_dump($o->self === $o);
$a = [1, [2, 3]]; $b = [];
$b[0] = &$a[1][0]; $b[0] = 'changed'; var_dump($a[1][0]);
$a[2] = &$a[0]; $a[0] = 'zero'; var_dump($a[2]);
$r = &O::$s; $r = 'via ref'; var_dump(O::$s);
$v = 'v'; O::$s = &$v; $v = 'bound'; var_dump(O::$s);
O::$s = &$o->p; $o->p = 'prop'; var_dump(O::$s);
$z = &$o->arr[1]; $z = 'elem'; var_dump($o->arr);
$g = 1;
function &getRef() { global $g; return $g; }
function getVal() { global $g; return $g; }
$gr = &getRef(); $gr = 99; var_dump($g);
$gv = &getVal(); $gv = 100; var_dump($g);
$plain = getRef(); $plain = 5; var_dump($g);
class Store { private $data = ['k' => 1]; function &get($k) { return $this->data[$k]; } function dump() { var_dump($this->data); } }
$s = new Store; $ref = &$s->get('k'); $ref = 'set via ref'; $s->dump();
var_dump(strlen(getRef()));
function &counter() { static $c = 0; $c++; return $c; }
$c1 = &counter(); $c1 += 10; var_dump(counter());

<?php
class A { public static $s = 'A'; public static function who() { return static::class . "/" . self::class . "/" . static::$s; }
  public static function call() { return static::who(); } public function chain() { return $this; } }
class B extends A { public static $s = 'B'; public static function test() { return parent::call(); } }
echo A::who(), " ", B::who(), " ", B::test(), " ", B::call(), "\n";
class C extends A {}
C::$s = 'C!'; echo A::$s, " ", C::$s, "\n";
$b = new B; var_dump($b->chain()->chain() === $b);
class P1 { private $v = 'p1'; public function p1v() { return $this->v; } }
class P2 extends P1 { private $v = 'p2'; public function p2v() { return $this->v; } }
$p = new P2; echo $p->p1v(), $p->p2v(), "\n";
var_dump($p); print_r($p); echo "\n"; var_export($p); echo "\n";
class Pr { protected $x = 1; } class Sib extends Pr { function peek(Pr $o) { return $o->x; } }
echo (new Sib)->peek(new Pr), "\n";
$o = new stdClass; $o->self = $o; var_dump($o); print_r($o); echo "\n";
$arr = [new stdClass, new stdClass]; $arr[0]->n = 1; var_dump($arr);
function mk() { return new stdClass; } $x = mk(); $y = mk(); var_dump($x, $y);
unset($x); $z = new stdClass; var_dump($z);
$q1 = new stdClass; $q1->a = 1; $q2 = new stdClass; $q2->a = 2; var_dump($q1 < $q2, $q1 == $q2);
$q2->a = 1; var_dump($q1 == $q2);
echo get_class($b), "\n";

<?php
$arr = [1, [2, 3], 'k' => ['x' => 4]];
list(&$a, list(, &$b)) = $arr;
['k' => ['x' => &$c]] = $arr;
$a = 10; $b = 30; $c = 40;
var_dump($arr);
$z = [];
[[&$new]] = $z;
$new = 'made';
var_dump($z);
list(,,list(&$v)) = $arr;
class A { public $v = 1;
  function f() { $r = &$this; $r->v = 2; $r = 5; var_dump($this->v); $a = [&$this]; var_dump($a[0] === $this); return array(&$this, 'g'); }
  function g() { return 'g'; } }
$x = new A; $cb = $x->f(); echo $cb(), "\n";
$f = function () { $a = &$this; return $a; };
var_dump($f->call(new A) instanceof A);

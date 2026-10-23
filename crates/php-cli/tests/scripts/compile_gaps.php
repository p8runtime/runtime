<?php
$a = 1; $b = 2;
$arr = [&$a, 'k' => &$b, 3];
$a = 10; $arr['k'] = 20;
var_dump($arr, $b);
$self = [1]; $self[] = &$self; echo count($self), "\n";
$x = ['y' => 1]; $r = array('a' => 1, &$x['y']); $r[0] = 'via'; var_dump($x);
class C { public $a = [1, 2, 3]; public $person = ['n' => 'x', 'm' => 'y']; public static $s = 5; protected static $p = 1; public static $nul = null; }
$c = new C;
unset($c->a[1], $c->person['n']);
var_dump($c);
$o = (object) ['a' => 1, 'b' => [2], 5 => 'five'];
var_dump($o, $o->a, (object) 1, (object) null, (object) 'str', (object) $c === $c);
var_dump(json_encode((object) ['f' => 1.5]));
var_dump(isset(C::$s), isset(C::$p), isset(C::$nul), isset(C::$nope));
$n = null; var_dump(isset($n?->foo), isset($c?->a));

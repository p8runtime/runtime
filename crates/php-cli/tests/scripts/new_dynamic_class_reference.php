<?php
// new $o->name(), new $a['k'](), new $c::$p(): the class name is the property or element; the parentheses are the constructor's arguments.
class A { public $n = 'B'; public static $s = 'B'; public $arr = ['k' => 'B']; public $o; function __construct(...$a) { echo 'A(', implode(',', $a), ")\n"; } function mk() { return 'B'; } }
class B { function __construct(...$a) { echo 'B(', implode(',', $a), ")\n"; } }
$a = new A;
$a->o = $a;
var_dump(get_class(new $a->n()), get_class(new $a->n(1, 2)), get_class(new $a->n));
var_dump(get_class(new $a->o->n(3)), get_class(new $a->arr['k'](4)), get_class(new $a::$s(5)), get_class(new A::$s(6)));
$c = 'B'; var_dump(get_class(new $c(7)), get_class(new ($a->mk())(8)));
$m = ['x' => 'A']; var_dump(get_class(new $m['x'](9)));

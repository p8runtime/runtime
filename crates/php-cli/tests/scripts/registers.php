<?php
// Results written straight into argument registers, and temporaries reused
// by the instruction that reads them: every shape where a result register
// is also an operand's or an argument's.
function id($x) { return $x; }
function add($a, $b) { return $a + $b; }
function pick($c, $a, $b) { return $c ? $a : $b; }
function three($a, $b, $c) { return "$a-$b-$c"; }
function fib($n) { return $n < 2 ? $n : fib($n - 1) + fib($n - 2); }

var_dump(add(id(1), id(2)));
var_dump(add(add(1, 2), add(3, 4)));
var_dump(id(id(id(5))) + id(6) * id(7));
var_dump(three(id('a') . 'b', add(1, 2) > 2 ? 'big' : 'small', -id(3)));
var_dump(pick(id(0) == 0, id('t'), id('f')));
var_dump(fib(15));
$a = [10, 20, 30];
var_dump($a[id(1)] + $a[add(1, 1)]);
var_dump(id($a)[2]);
var_dump(strlen(str_repeat('x', add(2, 3))) . '!');
var_dump((string) add(1, 2) . (int) '7z' . !id(false));
$i = 5;
var_dump($i > 3 ? $i - 1 : $i + 1, $i < 3 ? 'lt' : ($i == 5 ? 'eq' : 'gt'));
var_dump(id(1) ?: id(2), id(0) ?: id(3));
$x = 2;
$x = add($x, $x * $x) - id($x);
var_dump($x);
var_dump(count([id(1), add(1, 1), $x]), max_of(1, 9, 4));
function max_of($a, $b, $c) { return $a > $b ? ($a > $c ? $a : $c) : ($b > $c ? $b : $c); }

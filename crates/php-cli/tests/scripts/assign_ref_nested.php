<?php
// `$a = &$b` inside a larger expression is still a reference assignment.
class C {
    public $m = [];
    function f($n) {
        if (null === $i = &$this->m[$n]) {
            $i = "made $n";
        }
        return $i;
    }
}
$c = new C;
var_dump($c->f('a'), $c->f('a'), $c->m);
$x = 1;
$s = 'v' . ($y = &$x);
$y = 2;
var_dump($s, $x);

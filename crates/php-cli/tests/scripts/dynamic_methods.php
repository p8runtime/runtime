<?php
class A {
    private $v = 1;
    function get($x = 0) { return $this->v + $x; }
    private function secret() { return 'secret'; }
    static function make($n) { return "made $n"; }
    function viaSelf($m) { return [$this->$m(), self::$m(5), static::$m(6)]; }
    function hidden() { $m = 'secret'; return $this->$m(); }
}
$a = new A;
$m = 'get';
var_dump($a->$m(), $a->$m(10), $a->{'g' . 'et'}(2));
$s = 'make';
var_dump(A::$s(3), $a->viaSelf('get'), $a->hidden());
$args = [7];
var_dump($a->$m(...$args));
try { $p = 'secret'; $a->$p(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $u = 'nope'; $a->$u(); } catch (Error $e) { echo $e->getMessage(), "\n"; }
$cls = 'A';
var_dump($cls::$s(9));

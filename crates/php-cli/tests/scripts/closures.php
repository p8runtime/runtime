<?php
$mul = 3;
$f = function($x) use ($mul) { return $x * $mul; };
echo $f(5), "\n";
$mul = 10;
echo $f(5), "\n";
$count = 0;
$inc = function() use (&$count) { $count++; };
$inc(); $inc();
echo $count, "\n";
$g = fn($x) => $x + $mul;
echo $g(1), "\n";
$add = fn($a) => fn($b) => $a + $b;
echo $add(2)(3), "\n";
echo implode(",", array_map(fn($v) => $v * 2, [1, 2, 3])), "\n";
print_r(array_map(null, [1, 2], ['a', 'b']));
print_r(array_filter([1, 0, 2, null, 3]));
print_r(array_filter(['a' => 1, 'b' => 2, 'c' => 3], fn($k) => $k != 'b', ARRAY_FILTER_USE_KEY));
echo array_reduce([1, 2, 3, 4], fn($c, $v) => $c + $v, 10), "\n";
$arr = [3, 1, 2];
usort($arr, fn($a, $b) => $a <=> $b);
print_r($arr);
$assoc = ['x' => 3, 'y' => 1, 'z' => 2];
uasort($assoc, function($a, $b) { return $b - $a; });
print_r($assoc);
uksort($assoc, fn($a, $b) => strcmp($a, $b));
print_r($assoc);
$w = [1, 2, 3];
array_walk($w, function(&$v, $k, $p) { $v = "$p$k:$v"; }, '#');
print_r($w);
function named($a, $b) { return "$a-$b"; }
echo call_user_func('named', 1, 2), " ", call_user_func_array('named', [3, 4]), "\n";
$fname = 'named';
echo $fname('x', 'y'), "\n";
echo call_user_func('strtoupper', 'abc'), "\n";
class C {
    private $v = 'priv';
    public static function st($x) { return "st:$x"; }
    public function m($x) { return "m:$x:" . $this->v; }
    public function getClosure() { return function() { return $this->v; }; }
    public function __invoke($a) { return "invoked $a"; }
}
$c = new C;
echo call_user_func([$c, 'm'], 1), " ", call_user_func(['C', 'st'], 2), " ", call_user_func('C::st', 3), "\n";
echo $c->getClosure()(), "\n";
echo $c(7), " ", call_user_func($c, 8), "\n";
var_dump(is_callable('named'), is_callable('nope'), is_callable([$c, 'm']), is_callable($f), is_callable(42));
$fact = function($n) use (&$fact) { return $n <= 1 ? 1 : $n * $fact($n - 1); };
echo $fact(5), "\n";
echo (function() { return "iife"; })(), "\n";
$arr2 = [[2, 'b'], [1, 'a']];
usort($arr2, fn($x, $y) => $x[0] - $y[0]);
echo $arr2[0][1], "\n";
try { array_map(function($v) { throw new Exception("in cb $v"); }, [9]); } catch (Exception $e) { echo $e->getMessage(), "\n"; }
echo "end\n";

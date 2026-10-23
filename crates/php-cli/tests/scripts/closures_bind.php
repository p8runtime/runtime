<?php
class P { private $secret = 's1'; public $pub = 'p'; }
$peek = function() { return $this->secret; };
$b = Closure::bind($peek, new P, P::class);
echo $b(), "\n";
$b2 = $peek->bindTo(new P, 'P');
echo $b2(), "\n";
echo $peek->call(new P), "\n";
$len = strlen(...);
echo $len("hello"), "\n";
$fc = Closure::fromCallable('strtoupper');
echo $fc("x"), "\n";
$st = static function() { return isset($this) ? "has" : "none"; };
echo $st(), "\n";
function outer() { return array_map(fn($x) => $x . __FUNCTION__, ['a']); }
print_r(outer());
$c = function($a, $b = 2) {};
var_dump($c instanceof Closure);
$x = 5;
$d = function() use ($x) { return $x; };

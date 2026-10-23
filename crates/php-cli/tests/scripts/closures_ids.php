<?php
class P { private $secret = 's1'; }
$peek = function() { return $this->secret; };
echo spl_object_id($peek), "\n";
$b = Closure::bind($peek, new P, P::class);
echo spl_object_id($b), "\n";
$b2 = $peek->bindTo(new P, 'P');
echo spl_object_id($b2), "\n";
echo $peek->call(new P), "\n";
$len = strlen(...);
echo spl_object_id($len), "\n";
$fc = Closure::fromCallable('strtoupper');
echo spl_object_id($fc), "\n";
$st = static function() { return 1; };
echo spl_object_id($st), "\n";

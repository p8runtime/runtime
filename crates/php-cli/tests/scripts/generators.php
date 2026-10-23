<?php
function counter($n) { for ($i = 1; $i <= $n; $i++) { yield $i; } return "done"; }
foreach (counter(3) as $k => $v) echo "$k=$v ";
echo "\n";
$g = counter(2);
echo $g->current(), $g->key(), "\n";
$g->next(); echo $g->current(), "\n";
$g->next(); var_dump($g->valid(), $g->current()); echo $g->getReturn(), "\n";
function kv() { yield 'a' => 1; yield 'b' => 2; yield 10 => 'x'; yield 'y'; }
foreach (kv() as $k => $v) echo "$k:$v ";
echo "\n";
function echoer() { while (true) { $x = yield; if ($x === null) return; echo "got $x\n"; } }
$e = echoer(); $e->current(); $e->send('hello'); $e->send('world');
function acc() { $total = 0; while (true) { $v = yield $total; if ($v === null) break; $total += $v; } return $total; }
$a = acc(); echo $a->current(), " "; echo $a->send(5), " "; echo $a->send(10), "\n";
function inner() { yield 1; yield 2; return 3; }
function outer() { $r = yield from inner(); yield $r; yield from ['x' => 'X', 'y' => 'Y']; }
foreach (outer() as $k => $v) echo "$k=>$v ";
echo "\n";
function thrower() { try { yield 1; } catch (Exception $e) { echo "caught ", $e->getMessage(), "\n"; yield 99; } }
$t = thrower(); $t->current(); echo $t->throw(new Exception("boom")), "\n";
function lazy() { echo "start\n"; yield 1; }
$l = lazy(); echo "made\n"; foreach ($l as $v) echo $v, "\n";
var_dump($l instanceof Traversable, $l instanceof Iterator, $l instanceof Generator);
try { $l->rewind(); foreach ($l as $v) {} } catch (Exception $e) { echo get_class($e), ": ", $e->getMessage(), "\n"; }
$gen = (function() { yield 1; yield 2; })();
echo implode(",", iterator_to_array($gen)), "\n";
class Coll implements IteratorAggregate { private $items = [1, 2]; function getIterator(): Iterator { yield from $this->items; } }
foreach (new Coll as $v) echo $v; echo "\n";
class It implements Iterator { private $i = 0; function current(): mixed { return $this->i * 10; } function key(): mixed { return $this->i; } function next(): void { $this->i++; } function rewind(): void { $this->i = 0; } function valid(): bool { return $this->i < 3; } }
foreach (new It as $k => $v) echo "$k:$v "; echo "\n";
class Pub { public $a = 1; protected $b = 2; public $c = 3; }
foreach (new Pub as $k => $v) echo "$k=$v "; echo "\n";
function fib() { $a = 0; $b = 1; while (true) { yield $a; $t = $a + $b; $a = $b; $b = $t; } }
foreach (fib() as $i => $f) { if ($i >= 10) break; echo $f, " "; } echo "\n";

<?php
function g($it) { $r = yield from $it; var_dump($r); yield 'k' => 'last'; }
foreach (g(new ArrayIterator(['a' => 1, 'b' => 2])) as $k => $v) echo "$k=$v\n";
class Agg implements IteratorAggregate { function getIterator(): Iterator { return new ArrayIterator([10, 20]); } }
foreach (g(new Agg) as $k => $v) echo "$k=$v\n";
function h() { yield 1; yield from new ArrayObject([5, 6]); yield 2; }
foreach (h() as $k => $v) echo "$k=$v\n";
$gen = g(new ArrayIterator([7]));
echo $gen->current(), "\n"; $gen->send('x'); echo $gen->key(), "\n";

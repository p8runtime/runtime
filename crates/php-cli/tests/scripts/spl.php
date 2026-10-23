<?php
$ao = new ArrayObject(['b' => 2, 'a' => 1, 3]);
var_dump($ao, count($ao), $ao['a'], isset($ao['b']), isset($ao['zz']));
$ao['c'] = 5; $ao[] = 'appended'; unset($ao['b']);
foreach ($ao as $k => $v) echo "$k=$v ";
echo "\n";
$ao->ksort(); var_dump($ao->getArrayCopy());
$ao->uasort(fn($x, $y) => strcmp((string) $x, (string) $y)); var_dump(array_keys($ao->getArrayCopy()));
var_dump($ao['missing']);
$old = $ao->exchangeArray([9]); var_dump(count($old), $ao->getArrayCopy());
$it = new ArrayIterator([10, 20, 30]);
foreach ($it as $k => $v) echo "$k:$v ";
echo "\n";
$it->seek(1); var_dump($it->current());
try { $it->seek(10); } catch (OutOfBoundsException $e) { echo $e->getMessage(), "\n"; }
$r = new RecursiveArrayIterator([1, [2, 3], 4]);
foreach ($r as $k => $v) { echo $k, ' ', var_export($r->hasChildren(), true), "\n"; }
$fa = new SplFixedArray(3); $fa[0] = 'a'; $fa[2] = 'c';
var_dump($fa, $fa->getSize(), $fa->toArray());
try { $fa[5] = 1; } catch (RuntimeException $e) { echo get_class($e), ': ', $e->getMessage(), "\n"; }
foreach ($fa as $i => $v) echo "$i=", var_export($v, true), " ";
echo "\n";
$f2 = SplFixedArray::fromArray([1 => 'x', 3 => 'y']); var_dump($f2->toArray());
$s = new SplObjectStorage; $o1 = new stdClass; $o2 = new stdClass;
$s->attach($o1, 'one'); $s[$o2] = 'two'; var_dump(count($s), $s->contains($o1), $s[$o2]);
var_dump($s);
foreach ($s as $i => $o) { echo $i, ' ', $s->getInfo(), "\n"; }
$s->detach($o1); var_dump(count($s));
$st = new SplStack; $st->push(1); $st->push(2); $st->push(3);
var_dump($st->pop(), $st->top(), count($st));
foreach ($st as $k => $v) echo "$k=>$v ";
echo "\n";
var_dump($st);
$q = new SplQueue; $q->enqueue('a'); $q->enqueue('b'); var_dump($q->dequeue(), count($q));
$l = new SplDoublyLinkedList; $l->push('x'); $l->push('y'); $l->unshift('w'); $l->add(1, 'ins');
foreach ($l as $k => $v) echo "$k=$v ";
echo "\n";
try { $l->offsetGet(10); } catch (OutOfRangeException $e) { echo $e->getMessage(), "\n"; }
try { (new SplStack)->pop(); } catch (RuntimeException $e) { echo $e->getMessage(), "\n"; }
$h = new SplMinHeap; foreach ([5, 1, 3, 2] as $x) $h->insert($x);
var_dump($h->top(), count($h)); foreach ($h as $v) echo $v, ' '; echo "\n";
$mh = new SplMaxHeap; foreach ([5, 1, 9] as $x) $mh->insert($x); var_dump($mh->extract(), count($mh));
$pq = new SplPriorityQueue; $pq->insert('lo', 1); $pq->insert('hi', 10); $pq->insert('mid', 5);
var_dump($pq->extract()); $pq->setExtractFlags(SplPriorityQueue::EXTR_BOTH); var_dump($pq->top());
class RevHeap extends SplHeap { protected function compare($a, $b): int { return strlen($a) - strlen($b); } }
$rh = new RevHeap; foreach (['aaa', 'a', 'aa'] as $x) $rh->insert($x); foreach ($rh as $v) echo $v, ' '; echo "\n";
var_dump(class_implements(new ArrayObject), class_parents('SplStack'));
var_dump(iterator_apply(new ArrayIterator([1, 2, 3]), fn() => true));

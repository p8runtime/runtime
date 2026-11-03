<?php
// SplDoublyLinkedList and SplObjectStorage serialize()/unserialize() (their own string
// forms) and __serialize/__unserialize: a subclass is no longer left abstract
// (Composer's RuleWatchChain extends SplDoublyLinkedList).
class Chain extends SplDoublyLinkedList {}
$c = new Chain; $c->push('a'); $c->push(['b' => 1]); $c->setIteratorMode(SplDoublyLinkedList::IT_MODE_LIFO);
var_dump($c->serialize(), unserialize(serialize($c)), $c->__serialize());
$d = new SplDoublyLinkedList; $d->unserialize($c->serialize()); var_dump(count($d), $d->top());
$q = new SplQueue; $q->enqueue(1); var_dump(serialize($q));
$s = new SplObjectStorage; $o = new stdClass; $s[$o] = 'data'; var_dump(strlen(serialize($s)) > 0, count(unserialize(serialize($s))));
var_dump((new ReflectionClass('SplDoublyLinkedList'))->isAbstract(), (new ReflectionClass('Chain'))->isInstantiable());
class Store extends SplObjectStorage { public $extra; }
$s2 = new Store; $a = new stdClass; $a->n = 1; $s2[$a] = ['x']; $s2->attach(new DateTimeZone('UTC')); $s2->extra = 'e';
var_dump($s2->serialize(), serialize($s2));
$r = unserialize(serialize($s2)); var_dump(count($r), $r->extra);
$r2 = new Store; $r2->unserialize($s2->serialize()); var_dump(count($r2));

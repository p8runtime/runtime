<?php
// SplDoublyLinkedList's serialization: Serializable's methods and the magic
// ones, and a subclass (Composer's RuleWatchChain) that must be concrete.
class Chain extends SplDoublyLinkedList {
    public $extra = 'x';
}
$l = new SplDoublyLinkedList();
$l->push(1);
$l->push("two");
$l->push([3]);
var_dump($l->serialize());
var_dump($l->__serialize());
$s = serialize($l);
var_dump($s);
$back = unserialize($s);
var_dump(count($back), $back[1], $back->getIteratorMode());
$c = new Chain();
$c->push(4.5);
$c->setIteratorMode(SplDoublyLinkedList::IT_MODE_LIFO);
var_dump(serialize($c));
$c2 = unserialize(serialize($c));
var_dump(get_class($c2), $c2->extra, $c2->top(), $c2->getIteratorMode());
$st = new SplStack();
$st->push('a');
var_dump(serialize($st));
$q = new SplQueue();
$q->unserialize('i:4;:i:7;:s:1:"z";');
var_dump(count($q), $q->dequeue(), $q->serialize());
try {
    $q->__unserialize([1]);
} catch (UnexpectedValueException $e) {
    echo $e->getMessage(), "\n";
}

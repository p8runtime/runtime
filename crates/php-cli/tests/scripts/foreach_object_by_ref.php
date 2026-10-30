<?php
// foreach by reference over an object: its visible properties, each made a
// reference (json-schema walks decoded JSON this way); ArrayObject/ArrayIterator
// by their storage; an IteratorAggregate by its iterator; a user Iterator refused.
$o = json_decode('{"a":1,"b":{"c":2},"enum":[3]}');
foreach ($o as $k => &$v) { if (is_int($v)) $v *= 10; $seen[] = $k; }
unset($v);
var_dump($o, $seen);
class P { public $x = 1; protected $y = 2; private $z = 3; function bump() { foreach ($this as $k => &$v) { $v = "$k!"; } unset($v); return get_object_vars($this); } }
$p = new P; foreach ($p as &$v) { $v = 'out'; } unset($v);
var_dump($p, (new P)->bump());
$e = new stdClass; foreach ($e as &$v) { echo "never\n"; }
try { foreach (new ArrayIterator([1]) as &$v) {} } catch (Error $err) { echo get_class($err), ': ', $err->getMessage(), "\n"; }
$d = new stdClass; $d->a = 1; $d->b = 2;
foreach ($d as $k => &$v) { if ($k === 'a') unset($d->b); $v++; } unset($v);
var_dump(get_object_vars($d));
$ao = new ArrayObject([1, 2]); foreach ($ao as &$v) { $v *= 3; } unset($v); var_dump($ao->getArrayCopy());
class It implements IteratorAggregate { function getIterator(): Iterator { return new ArrayIterator([]); } }
try { foreach (new It as &$v) {} } catch (Error $err) { echo get_class($err), ': ', $err->getMessage(), "\n"; }
class UserIt implements Iterator { function current(): mixed { return 1; } function key(): mixed { return 0; } function next(): void {} function rewind(): void {} function valid(): bool { return false; } }
try { foreach (new UserIt as &$v) {} } catch (Error $err) { echo get_class($err), ': ', $err->getMessage(), "\n"; }

<?php
$ao = new ArrayObject(['a' => ['b' => 1]]);
$ao['a']['c'] = 2; $ao['a']['b']++; $ao['a'][] = 9; unset($ao['a']['b']);
var_dump($ao['a']);
$it = new ArrayIterator(['x' => []]); $it['x']['y'] = 1; var_dump($it['x']);
class Sub extends ArrayObject { function offsetGet(mixed $k): mixed { return parent::offsetGet($k); } }
$s = new Sub(['a' => []]); $s['a']['q'] = 1; var_dump($s['a']);
class Plain extends ArrayObject {} $p = new Plain(['a' => []]); $p['a']['q'] = 1; var_dump($p['a']);

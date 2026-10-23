<?php
class A { function m($v) { throw new LogicException("in method"); } static function s() { (new A)->m(1.5); } }
try { A::s(); } catch (Exception $e) { echo $e; echo "\n"; }
$e = new Exception("plain");
var_dump($e->getLine(), $e->getFile() === __FILE__, $e->getTrace());
$inner = new Exception("one");
throw new Exception("two", 0, $inner);

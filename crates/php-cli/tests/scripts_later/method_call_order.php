<?php
// PHP takes a method call's receiver and finds the method before the
// arguments run (#490).
class A { function m($x, $y) { return "A::m"; } }
class B { function m(&$x, $y) { $x = "changed"; return "B::m"; } }
function swap(&$o) { $o = new B; return 1; }

// 1. A later argument assigns the receiver's variable: the object the
// call started on is the receiver, and its method decides by-reference.
$o = new A; $v = "v";
echo $o->m($v, $o = new B), " v=$v\n";

// 2. A later argument replaces it through a reference.
$o = new A; $v = "v";
echo $o->m($v, swap($o)), " v=$v\n";

// 3. A call on null: the error, before any argument runs.
$n = null; $k = 0;
try { $n->m($k, $k = 5); } catch (Error $e) { echo $e->getMessage(), " k=$k\n"; }

// 4. A method that is not there: the error, before any argument runs.
class T { function m($x, $y) { return "T::m"; } }
function eff() { echo "eff "; return 1; }
$t = new T;
try { $t->nope(eff()); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { $t->nope($k, eff(), $k = 7); } catch (Error $e) { echo $e->getMessage(), " k=$k\n"; }

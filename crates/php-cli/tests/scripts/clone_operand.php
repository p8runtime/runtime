<?php
// clone takes the call, [...], -> and :: after it (`clone f()` clones what
// f returns: WordPress's `clone get_comment($id)`), but not ++ or any
// operator (`clone $o instanceof K` tests the clone).
function f() { $o = new stdClass; $o->made = 'by f'; return $o; }
class K { public $p; public $arr = []; static function m() { $o = new stdClass; $o->made = 'by K::m'; return $o; } function g() { return $this; } }
$k = new K; $k->p = new stdClass; $k->arr[0] = new ArrayObject([1]);
$a = clone f(); $b = clone K::m(); $c = clone $k->p; $d = clone $k->arr[0]; $e = clone (f()); $g = clone $k->g();
var_dump($a->made, $b->made, $c !== $k->p, get_class($d), $e->made, $g !== $k, clone $k instanceof K);

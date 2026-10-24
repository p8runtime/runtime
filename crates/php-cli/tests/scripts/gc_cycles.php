<?php
class N { public $o; function __destruct() { echo "d\n"; } }
$a = new N; $b = new N; $a->o = $b; $b->o = $a;
$w = WeakReference::create($a);
unset($a, $b);
var_dump($w->get() !== null);
var_dump(gc_collect_cycles());
var_dump($w->get());
$x = new stdClass; $x->self = $x; unset($x);
var_dump(gc_collect_cycles(), gc_status()["runs"], gc_status()["collected"]);

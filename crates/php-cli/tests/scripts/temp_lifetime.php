<?php
$o = new stdClass; $r = WeakReference::create($o);
unset($o); var_dump($r->get());
$o = new stdClass; $r = WeakReference::create($o);
var_dump($r->get() === $o);
unset($o); var_dump($r->get());
class D { function __destruct() { echo "destruct\n"; } }
function f() { return new D; }
var_dump(f() instanceof D);
echo "after\n";

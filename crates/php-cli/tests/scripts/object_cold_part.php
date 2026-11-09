<?php
// Dynamic properties and lazy state live outside the object (#396): an
// object reads as having none until one is written, and keeps them
// through a clone, a destructor and the cycle collector.
#[AllowDynamicProperties]
class O {
    public $a = 1;
    function __destruct() { echo "destruct ", json_encode(get_object_vars($this)), "\n"; }
}
#[AllowDynamicProperties]
class Plain { public $a = 1; public $b = [2]; }

$p = new Plain;
var_dump(isset($p->x), property_exists($p, 'x'), get_object_vars($p));
foreach ($p as $k => $v) echo "$k ";
echo "\n", json_encode($p), " ", serialize($p), "\n";
var_dump($p == new Plain, (array) $p);

$p->x = 'dyn';
$p->y = [1, 2];
unset($p->x);
var_dump(property_exists($p, 'x'), property_exists($p, 'y'), get_object_vars($p));

// A clone has its own copy; a clone of one without any has none.
$c = clone $p;
$c->y[] = 3;
$c->z = 'z';
var_dump($p->y, isset($p->z), get_object_vars($c));
$plain = clone new Plain;
var_dump(get_object_vars($plain));

// The destructor sees its dynamic properties.
$o = new O;
$o->extra = 'kept';
unset($o);
$o = new O;
unset($o);

// The cycle collector frees objects whose cycle runs through one.
$x = new Plain; $y = new Plain;
$x->other = $y; $y->other = $x;
unset($x, $y);
var_dump(gc_collect_cycles() >= 2);

// Lazy objects: their state lives there too.
$r = new ReflectionClass(Plain::class);
$ghost = $r->newLazyGhost(function (Plain $g) { echo "init\n"; $g->a = 10; });
var_dump($r->isUninitializedLazyObject($ghost));
$ghost->dyn = 'd';
var_dump($r->isUninitializedLazyObject($ghost), $ghost->a, $ghost->dyn);
$proxy = $r->newLazyProxy(function () { echo "factory\n"; $i = new Plain; $i->a = 20; return $i; });
var_dump($proxy->a, $r->isUninitializedLazyObject($proxy));
$r->resetAsLazyGhost($p, function (Plain $g) { echo "reinit\n"; });
var_dump($r->isUninitializedLazyObject($p));
var_dump(get_object_vars($p));
echo "end\n";

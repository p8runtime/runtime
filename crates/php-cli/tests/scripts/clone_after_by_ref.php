<?php
// A property once passed by reference (ksort($this->a), a by-ref argument)
// is a plain value again when nothing else refers to it: a clone gets its
// own copy (zval_add_ref). WordPress's tests back hooks up with clone after
// WP_Hook ksort()s its callbacks; sharing them leaked one test's filters
// into the next.
class H { public $a = [3 => 'x', 1 => 'y']; function s() { ksort($this->a); } }
$h = new H; $h->s(); $c = clone $h; $h->a[] = 'z';
var_dump(count($c->a), count($h->a));
function g(array &$a) { $a[] = 1; }
$o = new H; g($o->a); $c2 = clone $o; $o->a = [];
var_dump(count($c2->a));
// Still shared while something else refers to it.
$o2 = new H; $keep = &$o2->a; $c3 = clone $o2; $keep[] = 'w';
var_dump(count($c3->a), count($o2->a));
$o3 = new H; $o3->dyn = [1]; sort($o3->dyn); $c4 = clone $o3; $o3->dyn[] = 2;
var_dump(count($c4->dyn));

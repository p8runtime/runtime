<?php
// A function from an included file is not known when this file compiles:
// an element or property argument becomes a reference only if it takes one.
require __DIR__ . '/function_by_ref_late.inc';
class Holder { public $data = ['a' => 1]; function fill() { late_set($this->data, ['b', 'c'], 2); return $this->data; } }
echo json_encode((new Holder)->fill()), "\n";
$arr = ['x' => ['y' => 0]];
late_set($arr['x'], ['y'], 5);
echo json_encode($arr), "\n";
$h = new Holder;
echo late_value($h->data), ' ', json_encode($h->data), "\n";
echo late_value($arr['x']), ' ', json_encode($arr['x']), "\n";
$o = new stdClass; $o->list = [];
late_set($o->list, [0], 'first');
echo json_encode($o), "\n";

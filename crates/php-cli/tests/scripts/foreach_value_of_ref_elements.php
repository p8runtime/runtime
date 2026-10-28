<?php
// foreach by value over elements a by-reference foreach made references: the values, not the references.
$nodes = [['selector' => 'a'], ['selector' => null]];
foreach ($nodes as &$n) { $n['x'] = 1; }
unset($n);
foreach ($nodes as $m) { var_dump($m['selector']); }
class T { public $data = ['k' => [['selector' => 's']]]; function nodes() { foreach ($this->data['k'] as &$v) { $v['y'] = 2; } return $this->data['k']; } }
foreach ((new T)->nodes() as $m) { var_dump($m['selector']); }
function by_ref(array &$a) { foreach ($a as &$v) { $v['z'] = 3; } }
$arr = [['selector' => 'b']]; $o = new stdClass; $o->list = [['selector' => 'c']];
by_ref($arr); by_ref($o->list);
foreach ([$arr, $o->list] as $list) foreach ($list as $m) var_dump($m['selector']);
$copy = $o->list; foreach ($copy as $m) var_dump($m['selector'], $m['z']);

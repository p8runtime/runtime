<?php
// array_keys() with a filter value, null included, loose and strict.
$a = ['a', null, 'b', null, 0, '', false, '1', 1];
var_dump(array_keys($a, null, true));
var_dump(array_keys($a, null));
var_dump(array_keys($a, 1));
var_dump(array_keys($a, 1, true));
var_dump(array_keys(['x' => 'b', 'y' => 'B', 'z' => 'b'], 'b'));
var_dump(array_keys(['p' => 1, 'q' => 2]));
// WordPress's resolve_pattern_blocks(): the n-th null is where the n-th
// inner block goes.
$inner = ['<div>', null, 'text', null, '</div>'];
$nulls = array_keys($inner, null, true);
array_splice($inner, $nulls[1], 1, [null, null]);
var_dump($inner);

<?php
function double_value(&$v) {
    $v *= 2;
}

$arr = ['a' => ['b' => 1]];
array_walk_recursive($arr, 'double_value');
print_r($arr);

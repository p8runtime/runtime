<?php
function double_value(&$v) {
    $v *= 2;
}

$arr = [1, 2];
array_walk($arr, 'double_value');
print_r($arr);

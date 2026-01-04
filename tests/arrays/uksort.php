<?php
function compare_keys($a, $b) {
    return strcmp($a, $b);
}

$arr = ['b' => 2, 'a' => 1];
uksort($arr, 'compare_keys');
print_r($arr);

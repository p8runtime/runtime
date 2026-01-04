<?php
function compare_values($a, $b) {
    return $a <=> $b;
}

$arr = ['b' => 2, 'a' => 1];
uasort($arr, 'compare_values');
print_r($arr);

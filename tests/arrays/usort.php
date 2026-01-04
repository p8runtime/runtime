<?php
function compare_values($a, $b) {
    return $a <=> $b;
}

$arr = [2, 1];
usort($arr, 'compare_values');
print_r($arr);

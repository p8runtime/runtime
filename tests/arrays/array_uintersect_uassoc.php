<?php
function diff_values($a, $b) {
    return $a - $b;
}

function compare_keys($a, $b) {
    return strcmp($a, $b);
}

print_r(array_uintersect_uassoc(['a' => 1], ['a' => 1], 'diff_values', 'compare_keys'));

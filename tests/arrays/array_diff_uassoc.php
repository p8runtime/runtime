<?php
function compare_keys($a, $b) {
    return strcmp($a, $b);
}

print_r(array_diff_uassoc(['a' => 1], ['b' => 1], 'compare_keys'));

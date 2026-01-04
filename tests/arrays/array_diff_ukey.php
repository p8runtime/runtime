<?php
function compare_keys($a, $b) {
    return strcmp($a, $b);
}

print_r(array_diff_ukey(['a' => 1], ['b' => 1], 'compare_keys'));

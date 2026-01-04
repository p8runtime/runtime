<?php
function compare_keys($a, $b) {
    return strcmp($a, $b);
}

print_r(array_intersect_ukey(['a' => 1], ['a' => 1], 'compare_keys'));

<?php
function diff_values($a, $b) {
    return $a - $b;
}

print_r(array_udiff_assoc(['a' => 1], ['b' => 1], 'diff_values'));

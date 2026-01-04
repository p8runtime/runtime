<?php
function diff_values($a, $b) {
    return $a - $b;
}

print_r(array_uintersect_assoc(['a' => 1], ['a' => 1], 'diff_values'));

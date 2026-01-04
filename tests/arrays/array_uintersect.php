<?php
function diff_values($a, $b) {
    return $a - $b;
}

print_r(array_uintersect([1, 2], [2], 'diff_values'));

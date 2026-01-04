<?php
function diff_values($a, $b) {
    return $a - $b;
}

print_r(array_udiff([1, 2], [1], 'diff_values'));

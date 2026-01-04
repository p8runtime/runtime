<?php
function is_even($v) {
    return $v % 2 === 0;
}

print_r(array_filter([1, 2, 3], 'is_even'));

<?php
function greater_than_one($v) {
    return $v > 1;
}

print_r(array_find([1, 2, 3], 'greater_than_one'));

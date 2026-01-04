<?php
function is_two($v) {
    return $v === 2;
}

print_r(array_find_key(['a' => 1, 'b' => 2], 'is_two'));

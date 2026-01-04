<?php
function double_value($v) {
    return $v * 2;
}

print_r(array_map('double_value', [1, 2]));

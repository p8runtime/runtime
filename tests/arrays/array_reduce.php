<?php
function sum_reduce($carry, $item) {
    return $carry + $item;
}

echo array_reduce([1, 2], 'sum_reduce', 0) . "\n";

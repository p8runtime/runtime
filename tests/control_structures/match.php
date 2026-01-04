<?php
$value = 2;
$result = match ($value) {
    1 => 'one',
    2 => 'two',
    default => 'other',
};
echo "result: $result\n";

$color = match ('apple') {
    'apple' => 'red',
    'banana' => 'yellow',
    default => 'unknown',
};
echo "color: $color\n";

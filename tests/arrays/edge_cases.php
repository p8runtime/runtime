<?php
$arr = ['a' => 1, 'b' => 2, 'c' => 3];
print_r(array_slice($arr, -2, 1, true));

print_r(array_pad([1, 2], -4, 0));

echo array_search(0, ['0', 0, false]) . "\n";
var_export(array_search(0, ['0', 0, false], true));
echo "\n";

print_r(array_merge([1, 2], [3 => 4], ['a' => 5]));

print_r(array_unique(['1', 1, '01']));

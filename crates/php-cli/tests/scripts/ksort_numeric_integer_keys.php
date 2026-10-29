<?php
// ksort SORT_NUMERIC compares integer keys as integers (WordPress hook priorities PHP_INT_MAX and PHP_INT_MAX - 1).
$a = [PHP_INT_MAX => 'max', PHP_INT_MAX - 1 => 'max-1', 10 => 'ten', '9' => 'nine', 'x' => 'str'];
ksort($a, SORT_NUMERIC); var_dump(array_keys($a));
$b = [PHP_INT_MAX, PHP_INT_MAX - 1, 9007199254740993, 9007199254740992];
sort($b, SORT_NUMERIC); var_dump($b);
$c = [PHP_INT_MAX => 1, PHP_INT_MAX - 1 => 2]; ksort($c); var_dump(array_keys($c));
var_dump(PHP_INT_MAX <=> PHP_INT_MAX - 1, '9223372036854775807' <=> '9223372036854775806');

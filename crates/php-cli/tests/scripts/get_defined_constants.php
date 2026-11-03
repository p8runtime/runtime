<?php
// get_defined_constants: flat, and grouped by module (Core first, the script's as
// "user" in definition order); composer/pcre names PREG errors from the pcre group.
define('ZED', 1); const ALPHA = 'a'; define('MID', [1, 2]);
$all = get_defined_constants();
var_dump($all['E_ALL'], $all['PHP_EOL'], $all['ZED'], $all['PREG_INTERNAL_ERROR'], $all['GD_MAJOR_VERSION'] ?? 'no gd', isset($all['NOPE']));
$c = get_defined_constants(true);
var_dump($c['user'], array_keys($c)[0], $c['Core']['E_ALL'], $c['pcre']['PREG_BACKTRACK_LIMIT_ERROR']);
// composer/pcre: the name of a PREG error code.
$names = array_flip(array_filter($c['pcre'], fn($k) => str_ends_with($k, '_ERROR'), ARRAY_FILTER_USE_KEY));
var_dump($names[preg_last_error()] ?? null, $names[PREG_BAD_UTF8_ERROR]);

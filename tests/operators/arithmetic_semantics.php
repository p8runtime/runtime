<?php
// Integer overflow, exact division, int powers, numeric strings, and the
// errors PHP 8 throws for unsupported operands and division by zero.
var_dump("2.5" * 2, " 3 " + 1, "1e3" + 0, PHP_INT_MIN * -1, PHP_INT_MAX + 1, PHP_INT_MIN - 1);
var_dump(null + 1, true + 1, 7 % -3, -7 % 3, 2 ** 63, 2 ** -1, 2 ** 3, 10 / 4, 10 / 5, "5" % "3", -0.0 * 1);
var_dump(-"2.5", +"5", -PHP_INT_MIN, -(-3), +true);
foreach (['[] + 1', '"abc" * 1', '"abc" + 1', '1 % 0', '1 / 0', 'PHP_INT_MIN % -1', 'new stdClass + 1', 'null - []'] as $c) {
    try {
        var_dump(eval("return $c;"));
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}

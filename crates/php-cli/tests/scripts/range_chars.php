<?php
// range(): characters, ints, floats, steps, and PHP 8.3's argument rules.
function show($f) {
    try {
        $r = $f();
        echo json_encode($r), "\n";
    } catch (\Throwable $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
}
show(fn() => range('a', 'i'));
show(fn() => range('i', 'a'));
show(fn() => range('a', 'z', 5));
show(fn() => range('z', 'a', 7));
show(fn() => range('A', 'z', 13));
show(fn() => range('a', 'a'));
show(fn() => range('a', 'e', -1));
show(fn() => range('e', 'a', -2));
show(fn() => range('a', 'e', 10));
show(fn() => range("\xf0", "\xff", 4));
show(fn() => range("\x05", "\x00", 2));
show(fn() => range('ab', 'e'));
show(fn() => range('a', 'eh'));
show(fn() => range('', 'c'));
show(fn() => range('a', 5));
show(fn() => range(1, 'e'));
show(fn() => range('5', 'e'));
show(fn() => range('a', '9'));
show(fn() => range('1', '9'));
show(fn() => range('1', '9', 2));
show(fn() => range('10', '1'));
show(fn() => range('a', 'e', 1.5));
show(fn() => range('a', 'e', 2.0));
show(fn() => range(0, 10));
show(fn() => range(10, 0, 3));
show(fn() => range(0, 10, -2));
show(fn() => range(10, 0, -2));
show(fn() => range(0, 1, 0.25));
show(fn() => range(1, 0, 0.3));
show(fn() => range(0.5, 3));
show(fn() => range('1.5', '4'));
show(fn() => range(1, 2, 0));
show(fn() => range(1, 5, 10));
show(fn() => range(1, INF));
show(fn() => range(NAN, 1));
show(fn() => range(1, 3, INF));
show(fn() => range(5, 5));
show(fn() => range(-3, 3, 2));
show(fn() => range(PHP_INT_MAX - 2, PHP_INT_MAX));
show(fn() => range(PHP_INT_MIN, PHP_INT_MIN + 2));
show(fn() => range(1, 3, PHP_INT_MIN));
// (The too-large error's text differs between the php:8.4-cli image and
// php-src 8.4.27's source, which the suite's range_bug70239_*.phpt check:
// p8 follows the source; those tests cover it.)
show(fn() => range(true, 3));
show(fn() => count(range(1, 1000)));
var_dump(range('x', 'z'), range(2.5, 3.5));

<?php
// Math functions: rounding, integer division, min/max, bases, floats.
var_dump(abs(-5), abs(-5.5), abs(PHP_INT_MIN), ceil(4.1), ceil(-4.1), ceil(5), floor(4.9), floor(-4.1));
foreach ([0.285, 1.955, 5.045, 5.055, -0.5, 2.5, 3.5, 1234.5678, -1234.5678, 1e20, 0.1 + 0.2] as $v) {
    echo round($v), " ", round($v, 2), " ", round($v, -2), " ", round($v, 1, PHP_ROUND_HALF_EVEN), " ", round($v, 0, PHP_ROUND_HALF_DOWN), " ", round($v, 0, PHP_ROUND_HALF_ODD), "\n";
}
var_dump(round(5), round(5, -1), round(15, -1), round(-15, -1), round("3.7"), round(2.5, 0, 5), round(-2.5, 0, 6), round(2.5, 0, 7), round(2.1, 0, 8));
var_dump(intdiv(7, 2), intdiv(-7, 2), fmod(7.5, 2), fmod(-7.5, 2), fdiv(1, 0), fdiv(-1, 0), pow(2, 10), pow(2, -1), pow(2, 63), pow("3", 2), pow(-8, 1/3));
var_dump(max(1, 2, 3), max([1, 5, 3]), max("apple", "banana"), max(1, "2", 3.5), max([1, 2], [1, 3]), min(1, -1), min([0, -0.0]), max("10", "9"), max("abc", 0), min("abc", 0));
var_dump(pi(), is_nan(NAN), is_finite(INF), is_infinite(-INF), sqrt(16), sqrt(-1), exp(1), log(M_E), log(8, 2), log(100, 10), log10(1000));
var_dump(sin(0), cos(0), round(tan(M_PI_4), 10), atan2(1, 1), hypot(3, 4), deg2rad(180), rad2deg(M_PI));
var_dump(decbin(10), decbin(-1), decoct(8), bindec("1010"), bindec("0b11"), octdec("777"), hexdec("ff"), hexdec("0xFF"), hexdec("7fffffffffffffff"), hexdec("ffffffffffffffff"));
var_dump(base_convert("ff", 16, 2), base_convert("255", 10, 36), base_convert("zz", 36, 10), base_convert("0", 2, 16));
var_dump(hexdec("fg"));

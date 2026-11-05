<?php
// A NAN used as a condition is true, with PHP 8.5's "unexpected NAN value
// was coerced to bool" warning: if, !, (bool), &&, ternary, and through a
// reference. Other values convert silently.
$n = NAN;
if ($n) { echo "if: true\n"; }
var_dump(!$n);
var_dump((bool) $n);
var_dump($n && true);
echo ($n ? "ternary: true" : "ternary: false"), "\n";
$r = &$n;
if ($r) { echo "ref: true\n"; }
foreach ([0.0, -0.0, 1.5, INF, 0, "", "0", "a", [], [0], null, false, true] as $v) {
    echo var_export($v, true), " => ", $v ? "true" : "false", "\n";
}

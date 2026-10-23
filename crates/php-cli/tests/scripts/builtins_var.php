<?php
// Variable functions: types, conversions, print_r, var_export.
$values = [null, true, false, 0, -5, 1.5, -0.0, 1e100, "", "0", "12", " 12 ", "12abc", "1e3", "abc", [], [1, "a" => [2, null]]];
foreach ($values as $v) {
    echo gettype($v), " ", get_debug_type($v), " ";
    echo (int) is_null($v), (int) is_bool($v), (int) is_int($v), (int) is_float($v), (int) is_string($v), (int) is_array($v), (int) is_scalar($v), (int) is_numeric($v), "\n";
}
foreach (["42", "0x1A", "012", "0b11", " 7 ", "-15", "99999999999999999999", "z"] as $s) {
    echo intval($s), " ", intval($s, 16), " ", intval($s, 8), " ", intval($s, 0), " ", intval($s, 36), "\n";
}
var_dump(intval(42.99), intval(true), intval(null), floatval("1.5abc"), floatval("abc"), strval(1.0), strval(0.1 + 0.2), boolval("0"), boolval([]), boolval("0.0"));
print_r(1); echo "\n";
print_r(1.0); echo "\n";
print_r(null); echo "|\n";
print_r(false); echo "|\n";
print_r("text\n");
print_r([1, 2, [3, [4]], "k" => "v", "e" => []]);
echo print_r([1], true);
var_export(null); echo "\n";
var_export(false); echo "\n";
var_export(PHP_INT_MIN); echo "\n";
var_export(1.0); echo "\n";
var_export(0.1); echo "\n";
var_export(-0.0); echo "\n";
var_export(1e100); echo "\n";
var_export(INF); echo "\n";
var_export(NAN); echo "\n";
var_export("it's a \\ test\0end"); echo "\n";
var_export([1, "a" => [true, null], "it's" => [], 5 => 1.5]); echo "\n";
echo var_export([[[]]], true), "\n";

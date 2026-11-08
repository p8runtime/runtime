<?php
// array_diff / array_intersect compare (string) $a === (string) $b: an
// int and its decimal string are one value, "01" and "1" are two.

$a = [1, "1", "01", 2.5, "2.5", true, null, "", "a", "A", "Array", 0, "0", false];
$b = ["1", 2.5, "", "A", "0"];
var_dump(array_diff($a, $b));
var_dump(array_intersect($a, $b));
var_dump(array_diff(["x" => "1", "y" => 1, "z" => "001"], [1]));
var_dump(array_intersect(["k" => "10", 10, "0x1"], ["10"], [10, "0x1"]));
var_dump(array_diff([1, 2, 3], []));
var_dump(array_diff([], [1]));
// Keys are kept, and only the first array's order.
var_dump(array_intersect([5 => "b", 2 => "a", "s" => "c"], ["c", "a", "b"]));

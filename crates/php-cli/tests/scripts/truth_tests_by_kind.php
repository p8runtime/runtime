<?php
// Conditions, === and == at every kind of value: the ones a branch decides
// from the kind alone (a bool, an int, null; === of two kinds) and the ones
// it must look at (floats, strings of both makes, arrays, objects), each
// held plainly and through a reference, and an undefined variable.

class Box
{
    public $v = 1;
}

function values()
{
    $long = "a string longer than twenty-two bytes";
    return [
        "true" => true,
        "false" => false,
        "0" => 0,
        "1" => 1,
        "-1" => -1,
        "null" => null,
        "0.0" => 0.0,
        "-0.0" => -0.0,
        "1.0" => 1.0,
        "NAN" => NAN,
        '""' => "",
        '"0"' => "0",
        '"0.0"' => "0.0",
        '" "' => " ",
        '"a"' => "a",
        '"1"' => "1",
        '"1e1"' => "1e1",
        '"10"' => "10",
        '"abc"' => "abc",
        '"ABC"' => "ABC",
        "long" => $long,
        // The same bytes in strings made as the script runs.
        'made ""' => substr("x", 1),
        'made "0"' => str_repeat("0", 1),
        'made "a"' => strtolower("A"),
        'made "10"' => strrev("01"),
        'made "abc"' => strtolower("ABC"),
        "made long" => str_repeat($long, 1),
        "made long." => substr($long, 0, -1) . ".",
        "[]" => [],
        "[0]" => [0],
        '["0"]' => ["0"],
        "[[]]" => [[]],
        "box" => new Box,
        "box2" => new Box,
        "object" => new stdClass,
    ];
}

// A condition, every way a branch or a value is made of one.
function cond($v)
{
    $r = "";
    if ($v) { $r .= "T"; } else { $r .= "F"; }
    if (!$v) { $r .= "n"; } else { $r .= "-"; }
    $n = 0;
    while ($v) { $r .= "w"; if (++$n == 2) { break; } }
    $n = 0;
    do { $n++; } while ($v && $n < 2);
    $r .= $n;
    $b = (bool) $v;
    $not = !$v;
    $r .= ($b ? "b" : "-") . ($not ? "!" : "-");
    $r .= ($v ? "t" : "f") . ($v && $n ? "a" : "-") . ($v || !$n ? "o" : "-") . ($v xor true ? "x" : "-");
    return $r;
}

function cond_ref(&$v)
{
    $r = "";
    if ($v) { $r .= "T"; } else { $r .= "F"; }
    if (!$v) { $r .= "n"; } else { $r .= "-"; }
    $n = 0;
    while ($v) { $r .= "w"; if (++$n == 2) { break; } }
    $n = 0;
    do { $n++; } while ($v && $n < 2);
    $r .= $n;
    $b = (bool) $v;
    $not = !$v;
    $r .= ($b ? "b" : "-") . ($not ? "!" : "-");
    $r .= ($v ? "t" : "f") . ($v && $n ? "a" : "-") . ($v || !$n ? "o" : "-") . ($v xor true ? "x" : "-");
    return $r;
}

// Two values compared with === and !==, as branches (both ways round)
// and as values.
function same($x, $y)
{
    $r = "";
    if ($x === $y) { $r .= "I"; } else { $r .= "."; }
    if ($x !== $y) { $r .= "N"; } else { $r .= "."; }
    $n = 0;
    do { $n++; } while ($x === $y && $n < 2);
    $r .= $n;
    $n = 0;
    while ($x !== $y) { if (++$n == 2) { break; } }
    $r .= $n;
    $a = $x === $y;
    $b = $x !== $y;
    return $r . (int) $a . (int) $b;
}

function same_ref(&$x, &$y)
{
    $r = "";
    if ($x === $y) { $r .= "I"; } else { $r .= "."; }
    if ($x !== $y) { $r .= "N"; } else { $r .= "."; }
    $n = 0;
    do { $n++; } while ($x === $y && $n < 2);
    $r .= $n;
    $a = $x === $y;
    return $r . (int) $a;
}

// One side through a reference, the other plain.
function same_one_ref(&$x, $y)
{
    $r = "";
    if ($x === $y) { $r .= "I"; } else { $r .= "."; }
    if ($y !== $x) { $r .= "N"; } else { $r .= "."; }
    return $r;
}

// The same with == and !=.
function loose($x, $y)
{
    $r = "";
    if ($x == $y) { $r .= "E"; } else { $r .= "."; }
    if ($x != $y) { $r .= "D"; } else { $r .= "."; }
    $n = 0;
    do { $n++; } while ($x == $y && $n < 2);
    $r .= $n;
    $c = $x == $y;
    $d = $x != $y;
    return $r . (int) $c . (int) $d;
}

function loose_ref(&$x, &$y)
{
    $r = "";
    if ($x == $y) { $r .= "E"; } else { $r .= "."; }
    if ($x != $y) { $r .= "D"; } else { $r .= "."; }
    $c = $x == $y;
    return $r . (int) $c;
}

function loose_one_ref(&$x, $y)
{
    $r = "";
    if ($x == $y) { $r .= "E"; } else { $r .= "."; }
    if ($y != $x) { $r .= "D"; } else { $r .= "."; }
    return $r;
}

// (An object against a number with == is left out: PHP converts the
// object, with a notice, and p8 does not yet.)
function loose_all($x, $y)
{
    $number = fn($v) => is_int($v) || is_float($v);
    if ((is_object($x) && $number($y)) || ($number($x) && is_object($y))) {
        return "-";
    }
    $rx = $x;
    $ry = $y;
    $one = $x;
    return loose($x, $y) . " " . loose_ref($rx, $ry) . " " . loose_one_ref($one, $y);
}

// Against literals, on either side.
function literals($x)
{
    $r = "";
    if ($x === null) { $r .= "n"; } else { $r .= "."; }
    if ($x === false) { $r .= "f"; } else { $r .= "."; }
    if ($x === true) { $r .= "t"; } else { $r .= "."; }
    if ($x === 0) { $r .= "0"; } else { $r .= "."; }
    if ($x === 1) { $r .= "1"; } else { $r .= "."; }
    if ($x === 0.0) { $r .= "d"; } else { $r .= "."; }
    if ($x === "") { $r .= "e"; } else { $r .= "."; }
    if ($x === "0") { $r .= "z"; } else { $r .= "."; }
    if ($x === "a") { $r .= "a"; } else { $r .= "."; }
    if ($x === "a string longer than twenty-two bytes") { $r .= "l"; } else { $r .= "."; }
    if ($x === []) { $r .= "r"; } else { $r .= "."; }
    $r .= " ";
    if ($x !== null) { $r .= "n"; } else { $r .= "."; }
    if ($x !== false) { $r .= "f"; } else { $r .= "."; }
    if ($x !== 0) { $r .= "0"; } else { $r .= "."; }
    if ($x !== "") { $r .= "e"; } else { $r .= "."; }
    if ($x !== "a") { $r .= "a"; } else { $r .= "."; }
    if (null === $x) { $r .= "N"; } else { $r .= "."; }
    if ("a" !== $x) { $r .= "A"; } else { $r .= "."; }
    if (1 === $x) { $r .= "1"; } else { $r .= "."; }
    $r .= " ";
    if ($x == null) { $r .= "n"; } else { $r .= "."; }
    if ($x == false) { $r .= "f"; } else { $r .= "."; }
    if ($x == true) { $r .= "t"; } else { $r .= "."; }
    if ($x == "") { $r .= "e"; } else { $r .= "."; }
    if ($x == "0") { $r .= "z"; } else { $r .= "."; }
    if ($x == "a") { $r .= "a"; } else { $r .= "."; }
    if ($x == "10") { $r .= "t"; } else { $r .= "."; }
    if ($x == "abc") { $r .= "c"; } else { $r .= "."; }
    if ($x != "ABC") { $r .= "C"; } else { $r .= "."; }
    if ("1e1" == $x) { $r .= "E"; } else { $r .= "."; }
    if ($x == []) { $r .= "r"; } else { $r .= "."; }
    return $r;
}

// What the library and the other statements make of the same pairs.
function found($x, $all)
{
    $r = in_array($x, $all, true) ? "S" : ".";
    $r .= in_array($x, ["a", "0", "", null, false, 0, 0.0, [], "abc"], true) ? "s" : ".";
    $k = array_search($x, $all, true);
    $r .= " " . ($k === false ? "-" : $k) . " " . count(array_keys($all, $x, true));
    $r .= " " . match (true) {
        $x === null => "null",
        $x === false, $x === true => "bool",
        $x === 0, $x === 1 => "int",
        $x === "a", $x === "0" => "str",
        default => "other",
    };
    $r .= " " . match ($x) {
        null => "null",
        false => "false",
        0 => "zero",
        0.0 => "float zero",
        "" => "empty",
        "0" => "string zero",
        "a" => "a",
        [] => "array",
        default => "other",
    };
    return $r;
}

function undefined()
{
    $r = "";
    if ($u) { $r .= "T"; } else { $r .= "F"; }
    if (!$u) { $r .= "n"; } else { $r .= "-"; }
    if ($u === null) { $r .= "I"; } else { $r .= "."; }
    if ($u !== null) { $r .= "N"; } else { $r .= "."; }
    if ($u == null) { $r .= "E"; } else { $r .= "."; }
    if ($u === false) { $r .= "f"; } else { $r .= "."; }
    if (0 === $u) { $r .= "0"; } else { $r .= "."; }
    if ($u === $w) { $r .= "U"; } else { $r .= "."; }
    $x = "a";
    if ($x === $u) { $r .= "a"; } else { $r .= "."; }
    if ($u == $x) { $r .= "A"; } else { $r .= "."; }
    return $r;
}

$all = values();

echo "== conditions\n";
foreach ($all as $name => $v) {
    $copy = $v;
    printf("%-12s %s %s\n", $name, cond($v), cond_ref($copy));
}

echo "== literals\n";
foreach ($all as $name => $v) {
    printf("%-12s %s\n", $name, literals($v));
}

echo "== pairs\n";
foreach ($all as $xn => $x) {
    echo $xn, "\n";
    foreach ($all as $yn => $y) {
        $rx = $x;
        $ry = $y;
        $one = $x;
        printf("  %-12s %s %s %s  %s\n", $yn, same($x, $y), same_ref($rx, $ry), same_one_ref($one, $y), loose_all($x, $y));
    }
}

echo "== a value with itself, and one variable twice\n";
foreach ($all as $name => $v) {
    $copy = $v;
    printf("%-12s %s %s  %s %s\n", $name, same($v, $copy), same_ref($v, $v), loose($v, $copy), loose_ref($v, $v));
}

echo "== found\n";
foreach ($all as $name => $v) {
    printf("%-12s %s\n", $name, found($v, $all));
}

echo "== undefined\n";
echo undefined(), "\n";

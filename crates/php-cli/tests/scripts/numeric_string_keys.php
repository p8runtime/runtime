<?php
// Numeric-string keys (ZEND_HANDLE_NUMERIC_STR) through every lookup path:
// literal keys and runtime strings, reads, isset, writes, unset,
// array_key_exists, small (scanned) and large (chained) tables; and the
// string-key write's copy-on-write and reference cases. Expected output
// recorded with PHP 8.6.

$cases = ["0", "1", "-1", "08", "-0", "00", "0x1", "1e3", " 1", "1 ", "+1", "-", "",
    "9223372036854775807", "9223372036854775808", "-9223372036854775808", "-9223372036854775809",
    "18446744073709551616", "12345678901234567890", "1.5", "a1", "1a", "-01", "--1", "0.0"];

function show($a) {
    foreach ($a as $k => $v) {
        echo var_export($k, true), " => ", var_export($v, true), "\n";
    }
}

// Keys built at run time (no literal: a fresh string each), in a small
// and in a large table.
foreach ([0, 12] as $pad) {
    echo "-- runtime keys, $pad padding entries\n";
    $a = [];
    for ($i = 0; $i < $pad; $i++) {
        $a["pad" . $i] = $i;
    }
    foreach ($cases as $c) {
        $k = "" . $c . "";
        $a[$k] = "v" . $c;
    }
    foreach ($cases as $c) {
        $k = implode("", [$c]);
        echo var_export($k, true), ": ", var_export($a[$k] ?? null, true),
            " isset=", var_export(isset($a[$k]), true),
            " exists=", var_export(array_key_exists($k, $a), true), "\n";
    }
    foreach ($a as $k => $v) {
        if (!is_string($k) || strncmp($k, "pad", 3) !== 0) {
            echo gettype($k), " ", var_export($k, true), "\n";
        }
    }
    // An int key and the string that names it are one key.
    var_dump(isset($a[1]), isset($a[-1]), $a[0] ?? null, isset($a[8]), $a[PHP_INT_MAX] ?? null, $a[PHP_INT_MIN] ?? null);
    unset($a["1"], $a["-0"], $a["08"]);
    var_dump(array_key_exists(1, $a), array_key_exists("-0", $a), array_key_exists("08", $a), array_key_exists("8", $a));
    echo count($a), "\n";
}

// Literal keys (constants in the code).
echo "-- literal keys\n";
$b = ["5" => "five", "05" => "oh-five", "-5" => "minus", "-0" => "minus zero", "x" => "ex"];
show($b);
$b["7"] = "seven";
$b["07"] = "oh-seven";
$b["-0"] = "again";
var_dump($b[5], $b["5"], $b["05"], isset($b["7"]), isset($b[7]), isset($b["07"]), isset($b[0]), $b["-0"], $b["x"]);
var_dump(isset($b["9223372036854775807"]), isset($b["nope"]));
$b["9223372036854775807"] = "max";
$b["9223372036854775808"] = "past max";
var_dump(array_keys($b));
unset($b["5"]);
var_dump(isset($b[5]), count($b));

// Insertion order: a string key equal to an existing int key replaces it in place.
echo "-- order\n";
$c = [10 => "a", "b" => "b", 20 => "c"];
$c["10"] = "A";
$c["b"] = "B";
$c["30"] = "d";
$c["030"] = "e";
show($c);

// A string-key write into a shared array separates it.
echo "-- copy on write\n";
$d = ["k" => 1, "l" => 2];
$e = $d;
$e["k"] = 10;
$e["m"] = 3;
$f = $d;
$f["1"] = "one";
show($d); show($e); show($f);
for ($i = 0; $i < 3; $i++) {
    $g = $d;
    $g["k"] = $i;
    $g["new$i"] = $i;
    echo count($g), " ", $g["k"], " ", $d["k"], "\n";
}

// A string-key write through an element that is a reference.
echo "-- references\n";
$x = 1;
$h = ["r" => &$x, "s" => 2];
$h["r"] = 5;
$h["s"] = 6;
var_dump($x, $h["r"], $h["s"]);
$i2 = $h;
$i2["r"] = 7;
var_dump($x, $h["r"]);

// Many string keys (a chained table) written, read back and overwritten.
echo "-- large\n";
$big = [];
for ($i = 0; $i < 100; $i++) {
    $big["key$i"] = $i;
    $big["$i"] = -$i;
}
$sum = 0;
for ($i = 0; $i < 100; $i++) {
    $sum += $big["key$i"] + $big[(string) $i];
    $big["key$i"] = $i * 2;
}
var_dump(count($big), $sum, $big["key99"], $big[99], array_key_first($big), array_key_last($big));

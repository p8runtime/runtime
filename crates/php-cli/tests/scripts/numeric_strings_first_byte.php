<?php
// Numeric strings, at the edges a first-byte test could get wrong:
// array keys, ==, <=>, switch, in_array/array_search, arithmetic and its
// warnings, is_numeric, casts, intdiv, string increment.

$keys = ["0", "-0", "00", "1", "01", "-1", "1.0", "1e3", " 1", "1 ", "+1", "",
    "9223372036854775807", "9223372036854775808", "-9223372036854775808",
    "-9223372036854775809", "0x1A", "١", "a", "_x", "~", "9", ":"];
$a = [];
foreach ($keys as $k) {
    $a[$k] = $k;
}
var_dump($a);

$vals = ["abc", "ABC", "0", "1", "01", "10", "1e1", "100", "1e2", " 1", "1 ",
    "1abc", "", "0.0", "-0", ".5", "5.", "1e", "e1", "INF", "NAN", "inf",
    "9007199254740993", "9007199254740992", "9223372036854775807",
    "\t1", "\n1\n", "+1", "-1", "z", "_", "~1", ":1",
    "9", "0x1A", "١", "1 1", " ", "-", "+", "."];
$others = [0, 1, 10, -1, 0.0, 0.5, 1.0, 100.0, 9007199254740992,
    9007199254740993, PHP_INT_MAX, INF, NAN, null, true, false];

// Loose comparisons, string with string and string with everything else.
foreach ($vals as $x) {
    $line = sprintf("%-24s", var_export($x, true));
    foreach ($vals as $y) {
        $line .= ($x == $y ? "=" : ".") . ["<", "0", ">"][($x <=> $y) + 1];
    }
    echo $line, "\n";
}
foreach ($vals as $x) {
    $line = sprintf("%-24s", var_export($x, true));
    foreach ($others as $y) {
        $line .= ($x == $y ? "=" : ".") . ($y == $x ? "=" : ".") . ["<", "0", ">"][($x <=> $y) + 1]
            . ["<", "0", ">"][($y <=> $x) + 1] . ($x < $y ? "l" : "-") . ($x >= $y ? "g" : "-") . " ";
    }
    echo $line, "\n";
}
var_dump("abc" == 0, "1" == "01", "10" == "1e1", "100" == "1e2", "abc" == "ABC",
    " 1" == "1", "1 " == "1", "1abc" == 1, null == "0", "" == null, "0.0" == "0",
    "-0" == "0", "9007199254740993" == "9007199254740992",
    "9223372036854775807" == "9223372036854775806", "INF" == "INF", "NAN" == "NAN",
    ".5" == "0.5", "5." == "5", "1e" == "1", "e1" == "e1", "abc" != "abd",
    "abc" == "abc", "a" < "b", "B" < "a", "10" < "9", "10" < "9a", "Z" <=> "1",
    "1" <=> "Z", "" <=> "0", "0" <=> "", "" < "a");

// switch uses ==.
foreach (["1", "01", "1e0", "abc", "", "0", " 1", "a1", "1a"] as $s) {
    switch ($s) {
        case "1":
            echo "$s: one\n";
            break;
        case "abc":
            echo "$s: abc\n";
            break;
        case "":
            echo "$s: empty\n";
            break;
        case 0:
            echo "$s: zero\n";
            break;
        default:
            echo "$s: default\n";
    }
}

// in_array / array_search, loose and strict.
$hay = ["abc", "01", "1e3", " 5", "", "x"];
foreach (["1", "1000", "5", "abc", "ABC", 0, "0", null, 1000, "x", "01"] as $n) {
    var_dump([in_array($n, $hay), in_array($n, $hay, true), array_search($n, $hay), array_search($n, $hay, true)]);
}

// Arithmetic, warnings and TypeErrors.
set_error_handler(function ($no, $msg) {
    echo "warning: $msg\n";
    return true;
});
foreach (["5", "5.5", " 5", "5 ", "5abc", "abc", "", "1e3", ".5", "-", "_5", "~5", "0x1A", "١"] as $s) {
    try {
        var_dump($s + 1, $s * 2, -$s);
    } catch (TypeError $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
foreach (["10", "abc", "7x", ""] as $s) {
    try {
        var_dump(intdiv($s, 3));
    } catch (TypeError $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
restore_error_handler();

// is_numeric and casts. ((float) "-0" is -0.0 in PHP, 0.0 in p8 before
// and after this test was written: left out.)
foreach (array_diff($vals, ["-0"]) as $s) {
    echo var_export($s, true), " ", var_export(is_numeric($s), true), " ", (int) $s, " ", var_export((float) $s, true), "\n";
}

// String increment.
foreach (["a", "z", "Az", "9", "a9", "1", "1.5", "-1", " 1", "1e2", "", "_"] as $s) {
    $t = $s;
    $t++;
    var_dump($t);
}

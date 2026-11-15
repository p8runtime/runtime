<?php
// str_replace/str_ireplace on strings (read in place, the result joined
// from its pieces up to four replacements, built in a buffer past that),
// beside the array and non-string forms; outputs recorded from PHP 8.6.
$s = "a quick brown fox";
var_dump(str_replace("q", "kw", $s));
var_dump(str_replace("o", "0", $s, $n), $n);
var_dump(str_replace("x", "", "xxxx", $n), $n);
var_dump(str_replace("x", "yz", "xxxx", $n), $n);
var_dump(str_replace("x", "yz", "xxxxx", $n), $n);
var_dump(str_replace("x", "-", "axbxcxdxexfxgxhx", $n), $n);
var_dump(str_replace("ab", "AB", str_repeat("ab", 20), $n), $n);
var_dump(str_replace("a", "long replacement text", "banana", $n), $n);
var_dump(str_replace("zz", "y", $s, $n), $n);
var_dump(str_replace("", "y", $s, $n), $n);
var_dump(str_replace("a", "b", "", $n), $n);
var_dump(str_replace("a", "", "a"));
var_dump(str_replace("ab", "c", "ab"));
var_dump(str_replace("Q", "kw", $s), str_ireplace("Q", "kw", $s, $n), $n);
var_dump(str_ireplace("O", "[o]", "Foo bOo", $n), $n);
var_dump(str_ireplace("QUICK", "slow", "A Quick quick QUICK qUiCk fox", $n), $n);
$t = str_repeat("short ", 3);
$u = str_replace("nothing", "here", $t);
var_dump($u === $t);
// A one-byte search at every place in subjects of 0 to 70 bytes (short
// ones are read a word at a time), one and two hits, with results short
// and long: each result's length and the count, and a digest of all.
$all = "";
$total = 0;
for ($len = 0; $len <= 70; $len++) {
    for ($at = 0; $at < max($len, 1); $at++) {
        $s = str_repeat("a", $len);
        if ($len > 0) {
            $s[$at] = "b";
        }
        foreach (["", "c", "dd", str_repeat("e", 9)] as $with) {
            $r = str_replace("b", $with, $s, $n);
            $r2 = str_replace("b", $with, $s . "b" . $s, $n2);
            $all .= $r . "|" . $r2 . "|";
            $total += $n + $n2;
        }
    }
}
var_dump(md5($all), strlen($all), $total);
// Non-string scalars convert as before.
var_dump(str_replace(1, 2, 1213, $n), $n);
var_dump(str_replace(1.5, true, "x1.5y", $n), $n);
var_dump(str_replace(null, "x", "abc", $n), $n);
var_dump(str_replace("a", null, "banana", $n), $n);
var_dump(str_replace("a", 7, 3.25));
// Arrays: each subject, each search with its replacement.
var_dump(str_replace(["a", "b"], ["b", "c"], "ab", $n), $n);
var_dump(str_replace(["a", "b"], "x", ["ab", "k" => "ba", 3 => "zz"], $n), $n);
var_dump(str_replace(["a", "n", "q"], ["1"], "banana", $n), $n);
var_dump(str_replace("a", "o", ["banana", 12, null], $n), $n);
try {
    str_replace("a", ["b"], "abc");
} catch (TypeError $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
// The count by reference, also into an existing variable of another type.
$c = "was a string";
str_replace("o", "0", "foo boo", $c);
var_dump($c);
$r = ["k" => 0];
str_replace("o", "0", "foo", $r["k"]);
var_dump($r);
// A Stringable subject goes through the conversion path.
class S { function __toString(): string { return "stringable q"; } }
var_dump(str_replace("q", "Q", new S, $n), $n);

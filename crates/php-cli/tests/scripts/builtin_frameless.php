<?php
// Builtins called through the frameless path (arguments plainly of their
// types) and through the checked one (a conversion, an object, a
// reference), in a namespace so the calls are real calls, not opcodes.
namespace N;

class D {
    public $s = "abc";
    function __destruct() { echo "destructed\n"; }
}
class S {
    function __toString(): string { return "str"; }
}

function trace_of(\Throwable $e): string {
    $f = $e->getTrace()[0];
    return $f['function'] . '@' . $f['line'];
}

// Plain: the result, and the arguments left as they were.
$s = "Hello";
var_dump(strlen($s), strtolower($s), strtoupper($s), $s);
var_dump(substr($s, 1, 3), substr($s, 0), str_contains($s, "ell"), str_starts_with($s, "He"), strpos($s, "l"), strrpos($s, "l"));
var_dump(trim("  x  "), ltrim("..x", "."), rtrim("x\n"), explode(",", "a,b,,c"), explode(",", "a,b,c", -1));
var_dump(count([1, 2, 3]), count([1, [2, 3]], COUNT_RECURSIVE), in_array("2", [1, 2, 3]), in_array("2", [1, 2, 3], true));
var_dump(array_key_exists("a", ["a" => null]), array_key_exists(1, [1 => 0]), array_search(3, [1, 2, 3]), array_keys(["x" => 1, "y" => 2]));
var_dump(abs(-5), abs(-2.5), abs(PHP_INT_MIN), is_int(1), is_string(1), is_array([]), is_null(null), intval("12abc"), intval("ff", 16));
var_dump(sprintf("%05d|%s|%.2f", 42, "x", 1.5), str_repeat("ab", 3), ucfirst("abc"), lcfirst("ABC"), ord("A"), chr(66));

// Checked: a conversion (string to int, int to string, float to int),
// then the same function plain again.
var_dump(str_repeat("x", "3"), strlen(12345), abs("-7"), substr("abcdef", 2.0));

// An object: its __toString() for a string parameter; inert for get_class.
var_dump(strlen(new S), strtolower(new S), get_class(new S), spl_object_id(new S) > 0);
// Countable: count() runs PHP code.
class C implements \Countable { function count(): int { echo "counted\n"; return 7; } }
var_dump(count(new C));

// A reference argument.
$r = "ref";
$alias = &$r;
var_dump(strlen($r), strtoupper($alias));

// Errors from a frameless call: the builtin is the trace's first frame,
// at the call's line.
try {
    explode("", "abc");
} catch (\ValueError $e) {
    echo get_class($e), ": ", $e->getMessage(), " ", trace_of($e), "\n";
}
try {
    strpos("abc", "a", 10);
} catch (\ValueError $e) {
    echo get_class($e), ": ", $e->getMessage(), " ", trace_of($e), "\n";
}
// The count is wrong: the checked path's ArgumentCountError.
try {
    strlen();
} catch (\ArgumentCountError $e) {
    echo $e->getMessage(), "\n";
}
// A type it cannot take.
try {
    strlen([]);
} catch (\TypeError $e) {
    echo $e->getMessage(), "\n";
}

// A deprecation and a warning raised inside a frameless call name it.
var_dump(ord(""));
var_dump(chr(300));

// The user error handler runs after the call, at its line.
set_error_handler(function ($no, $msg, $file, $line) {
    echo "handler: $msg @", $line, "\n";
    return true;
});
var_dump(ord("ab"));
restore_error_handler();

// An object argument to an inert builtin, then freed.
$d = new D;
var_dump(get_class($d), is_object($d));
unset($d);
echo "after\n";

// A call in a loop: the registers stay consistent.
$total = 0;
for ($i = 0; $i < 1000; $i++) {
    $total += strlen(str_repeat("a", $i % 7)) + abs(-$i) + count([$i]);
}
var_dump($total);

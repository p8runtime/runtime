<?php
// A builtin call whose arguments are variables or constants (the loop
// passes them in place, Op::CallRegs), and every way that falls back.

function lengths(array $words) {
    $n = 0;
    foreach ($words as $w) {
        $n += strlen($w) + strlen(trim($w));
    }
    return $n;
}
echo lengths([" a ", "bb", "ccc "]), "\n";

// Two parameters, consecutive registers, into a two-argument builtin.
function find($haystack, $needle) {
    return strpos($haystack, $needle);
}
var_dump(find("hello world", "o"), find("abc", "z"));

// The result into the argument's own variable.
function shout($s) {
    $s = strtoupper($s);
    $s = ucfirst(strtolower($s));
    return $s;
}
echo shout("hELLO"), "\n";

// A constant argument.
function lower_const() {
    return strtolower("MiXeD");
}
echo lower_const(), "\n";

// The variable is a reference: the argument is its value.
function by_ref() {
    $a = "Ref";
    $b = &$a;
    $r = strtoupper($b);
    $b = "changed";
    return [$r, $a];
}
var_dump(by_ref());

// Undefined: the warning, then the call with null.
function undefined_arg() {
    return @strlen($nope) . "|" . gettype(@intval($nope));
}
echo undefined_arg(), "\n";
function undefined_warns() {
    return intval($missing);
}
var_dump(undefined_warns());

// A coercion (an int where a string is taken): the checked path.
function coerced($i) {
    return strtoupper($i) . strlen($i);
}
echo coerced(12345), "\n";

// A builtin on the moved ABI: the variable is not consumed.
function keeps($a) {
    $v = array_values($a);
    $k = array_keys($a);
    return [count($a), $v, $k];
}
var_dump(keeps(["x" => 1, "y" => 2]));

// A user function with a variable argument: the moves run as they are.
function id($x) {
    return $x;
}
function calls_user($v) {
    return id($v) . id($v);
}
echo calls_user("u"), "\n";

// A jump into the call (the argument is a ternary).
function pick($c, $a, $b) {
    return strtoupper($c ? $a : $b);
}
echo pick(true, "yes", "no"), pick(false, "yes", "no"), "\n";

// A builtin that throws: the trace names it, at the call's line.
function repeat($s, $n) {
    return str_split($s, $n);
}
try {
    repeat("x", 0);
} catch (ValueError $e) {
    echo get_class($e), ": ", $e->getMessage(), " at line ", $e->getLine(), "\n";
    echo $e->getTraceAsString(), "\n";
}

// A warning from a frameless builtin.
function warns($haystack) {
    return strpos($haystack, "a", 10);
}
try {
    var_dump(warns("abc"));
} catch (ValueError $e) {
    echo $e->getMessage(), "\n";
}

// An object argument held only by the variable: the variable keeps it.
class D {
    function __destruct() {
        echo "D gone\n";
    }
}
function object_arg() {
    $o = new D;
    $name = get_class($o);
    echo "still here: ", $name, "\n";
    return spl_object_id($o) > 0;
}
var_dump(object_arg());

// In a loop.
function loop($n) {
    $t = 0;
    for ($i = 0; $i < $n; $i++) {
        $t += abs($i - 5);
    }
    return $t;
}
echo loop(10), "\n";
echo "done\n";

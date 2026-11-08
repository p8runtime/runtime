<?php
// A builtin call whose arguments are variables or constants (the loop
// passes them in place, Op::CallRegs), and every way that falls back.
// Everything runs twice: a call site resolves on its first run, so the
// second is the one passing its variables in place.

function lengths(array $words) {
    $n = 0;
    foreach ($words as $w) {
        $n += strlen($w) + strlen(trim($w));
    }
    return $n;
}

// Two parameters, consecutive registers, into a two-argument builtin.
function find($haystack, $needle) {
    $at = strpos($haystack, $needle);
    return [$at, $haystack, $needle];
}

// The result into the argument's own variable, and the argument used after.
function shout($s) {
    $t = strtoupper($s);
    $s = ucfirst(strtolower($s));
    return $t . "/" . $s;
}

// A constant argument.
function lower_const() {
    return strtolower("MiXeD");
}

// The variable is a reference: the argument is its value.
function by_ref() {
    $a = "Ref";
    $b = &$a;
    $r = strtoupper($b);
    $b = "changed";
    return [$r, $a];
}

// Undefined: the warning, then the call with null.
function undefined_arg() {
    return @strlen($nope) . "|" . gettype(@intval($nope));
}
function undefined_warns() {
    return intval($missing);
}

// A coercion (an int where a string is taken): the checked path.
function coerced($i) {
    return strtoupper($i) . strlen($i) . gettype($i);
}

// A builtin on the moved ABI: the variable is not consumed.
function keeps($a) {
    $v = array_values($a);
    $k = array_keys($a);
    return [count($a), $v, $k, $a];
}

// A user function with a variable argument: the moves run as they are.
function id($x) {
    return $x;
}
function calls_user($v) {
    return id($v) . id($v);
}

// A jump into the call (the argument is a ternary).
function pick($c, $a, $b) {
    return strtoupper($c ? $a : $b);
}

// A builtin that throws: the trace names it, at the call's line.
function repeat($s, $n) {
    return str_split($s, $n);
}

// A warning from a frameless builtin.
function warns($haystack) {
    return strpos($haystack, "a", 10);
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

// In a loop.
function loop($n) {
    $t = 0;
    for ($i = 0; $i < $n; $i++) {
        $t += abs($i - 5);
        $t += abs($i);
    }
    return $t;
}

function run() {
    echo lengths([" a ", "bb", "ccc "]), "\n";
    var_dump(find("hello world", "o"), find("abc", "z"));
    echo shout("hELLO"), "\n";
    echo lower_const(), "\n";
    var_dump(by_ref());
    echo undefined_arg(), "\n";
    var_dump(undefined_warns());
    echo coerced(12345), "\n";
    var_dump(keeps(["x" => 1, "y" => 2]));
    echo calls_user("u"), "\n";
    echo pick(true, "yes", "no"), pick(false, "yes", "no"), "\n";
    try {
        repeat("x", 0);
    } catch (ValueError $e) {
        echo get_class($e), ": ", $e->getMessage(), " at line ", $e->getLine(), "\n";
        echo $e->getTraceAsString(), "\n";
    }
    try {
        var_dump(warns("abc"));
    } catch (ValueError $e) {
        echo $e->getMessage(), "\n";
    }
    var_dump(object_arg());
    echo loop(10), "\n";
}

run();
echo "again\n";
run();
echo "done\n";

<?php
// A builtin's by-reference array parameter takes an array (or, where
// typed array|object, an object): an undefined variable (made null by the
// reference), null or a scalar is a TypeError, and the variable is left
// as it was (#579).

function attempt(string $f, callable $call): void {
    try {
        $r = $call();
        echo "$f: ", var_export($r, true), "\n";
    } catch (TypeError $e) {
        echo "$f: ", get_class($e), ": ", $e->getMessage(), "\n";
    }
}

$cmp = fn($a, $b) => $a <=> $b;
attempt("usort", function () use ($cmp) { return usort($u, $cmp); });
attempt("uasort", function () use ($cmp) { return uasort($u, $cmp); });
attempt("uksort", function () use ($cmp) { return uksort($u, $cmp); });
foreach (["sort", "rsort", "asort", "arsort", "ksort", "krsort", "natsort", "natcasesort", "shuffle", "array_pop", "array_shift", "end", "reset", "next", "prev"] as $f) {
    attempt($f, function () use ($f) { return $f($u); });
}
attempt("array_push", function () { return array_push($u, 1); });
attempt("array_unshift", function () { return array_unshift($u, 1); });
attempt("array_splice", function () { return array_splice($u, 0); });
attempt("array_walk", function () { return array_walk($u, fn($x) => 0); });
attempt("array_walk_recursive", function () { return array_walk_recursive($u, fn($x) => 0); });

// The variable is null afterwards (the reference made it), not an array.
try {
    sort($undefined);
} catch (TypeError $e) {
    echo $e->getMessage(), " on line ", $e->getLine(), "\n";
}
var_dump($undefined);

// Null, a string, an int, a float, a bool, an object.
$n = null;
attempt("sort null", function () use (&$n) { return sort($n); });
$s = "abc";
attempt("sort string", function () use (&$s) { return sort($s); });
var_dump($s);
$i = 5;
attempt("array_push int", function () use (&$i) { return array_push($i, 1); });
var_dump($i);
$f = 1.5;
attempt("end float", function () use (&$f) { return end($f); });
$b = false;
attempt("array_walk bool", function () use (&$b) { return array_walk($b, fn($x) => 0); });
$o = new stdClass();
attempt("usort object", function () use (&$o, $cmp) { return usort($o, $cmp); });
attempt("shuffle object", function () use (&$o) { return shuffle($o); });

// Arrays are taken as before.
$a = [3, 1, 2];
attempt("sort array", function () use (&$a) { return sort($a); });
echo implode(",", $a), "\n";
attempt("usort array", function () use (&$a) { return usort($a, fn($x, $y) => $y <=> $x); });
echo implode(",", $a), "\n";
attempt("array_push array", function () use (&$a) { return array_push($a, 9); });
echo implode(",", $a), "\n";
attempt("end array", function () use (&$a) { return end($a); });

// Through call_user_func_array, with the reference in the list.
$z = null;
$args = [&$z];
attempt("cufa sort", function () use ($args) { return call_user_func_array('sort', $args); });

// array_multisort names its first argument.
$x = 3;
try { array_multisort($x); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$x = "s";
try { array_multisort($x); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$a = [2, 1];
$x = "s";
try { array_multisort($a, $x); } catch (TypeError $e) { echo $e->getMessage(), "\n"; }
$x = 99;
try { array_multisort($a, $x); } catch (ValueError $e) { echo $e->getMessage(), "\n"; }

// array|object by value is named "array" too.
foreach (["current", "key", "pos"] as $f) {
    attempt($f, fn() => $f(5));
}
attempt("http_build_query", fn() => http_build_query("x"));

// Out parameters take anything.
$m = 5;
var_dump(preg_match('/b/', 'abc', $m), $m);
$c = "x";
var_dump(str_replace('a', 'b', 'aaa', $c), $c);

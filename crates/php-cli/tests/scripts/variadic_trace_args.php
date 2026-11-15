<?php
// A variadic parameter's arguments in a backtrace (#557): one by one, as
// they were passed, the named ones by their names.
function f(...$xs) { debug_print_backtrace(); var_dump(debug_backtrace()[0]["args"]); }
f(1, 2);
f();
function g($a, ...$xs) { debug_print_backtrace(); var_dump(count(debug_backtrace()[0]["args"])); }
g(1, 2, 3);
g(1);
function h(...$xs) { throw new Exception("e"); }
try {
    h("a", [1]);
} catch (Exception $e) {
    echo $e->getTraceAsString(), "\n";
    var_dump($e->getTrace()[0]["args"]);
}
function k(...$xs) { debug_print_backtrace(); var_dump(debug_backtrace()[0]["args"]); }
k(...["x" => 1, "y" => 2]);
k(1, y: 2);
function q(int ...$xs) { debug_print_backtrace(); }
q("5", 6.0);
function r(&...$xs) { $xs[0] = 3; debug_print_backtrace(); }
$a = 1;
$b = 2;
r($a, $b);
class C {
    function m(...$a) { echo (new Exception())->getTraceAsString(), "\n"; }
    static function st(string ...$a) { debug_print_backtrace(); }
}
(new C())->m(1, b: "two");
C::st("a", "b");
array_map(function (...$v) { debug_print_backtrace(); }, [1], [2]);

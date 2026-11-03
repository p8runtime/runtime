<?php
// A builtin's frame in a backtrace taken inside its callback or an
// autoloader it ran shows the line of the call to the builtin.
spl_autoload_register(function ($c) {
    $t = debug_backtrace();
    echo $t[1]['function'], ' ', $t[1]['line'], "\n";
});

var_dump(is_callable(["Nope", "m"]));
function f() {
    return array_map(
        function ($x) { $t = debug_backtrace(); return $t[1]['function'] . ' ' . $t[1]['line']; },
        [1]
    );
}
var_dump(f());
$a = [2, 1];
usort($a, function ($x, $y) { echo debug_backtrace()[1]['function'], ' ', debug_backtrace()[1]['line'], "\n"; return $x <=> $y; });

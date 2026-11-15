<?php
declare(strict_types=1);
// A callback call_user_func runs as the caller's own call
// (ZEND_INIT_USER_CALL) is checked as the caller's call: strict types
// here. The builtin's call (by a variable's name, or in a namespace by an
// unqualified name, resolved when it runs) is a builtin's, which are never
// strict.
namespace N {
    function show() { debug_print_backtrace(); }
    function typed(int $x) { return $x; }
    echo "-- unqualified\n";
    call_user_func('N\show');
    call_user_func_array('N\show', []);
    var_dump(call_user_func('N\typed', "1"));
    echo "-- qualified\n";
    \call_user_func('N\show');
    \call_user_func_array('N\show', []);
    try { var_dump(\call_user_func('N\typed', "2")); } catch (\TypeError $e) { echo $e->getMessage(), "\n", $e->getTraceAsString(), "\n"; }
}
namespace M {
    use function call_user_func;
    echo "-- imported\n";
    call_user_func('N\show');
    try { var_dump(call_user_func('N\typed', "3")); } catch (\TypeError $e) { echo $e->getMessage(), "\n"; }
}
namespace {
    function f(int $x) { return $x; }
    echo "-- global\n";
    try { var_dump(call_user_func('f', "4")); } catch (TypeError $e) { echo $e->getMessage(), "\n", $e->getTraceAsString(), "\n"; }
    try { var_dump(call_user_func_array('f', ["5"])); } catch (TypeError $e) { echo $e->getMessage(), "\n", $e->getTraceAsString(), "\n"; }
    var_dump(call_user_func('f', 6));
    $name = 'call_user_func';
    var_dump($name('f', "7"));
    var_dump(call_user_func('strlen', 'abc'));
}

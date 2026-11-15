<?php
// call_user_func_array() passes the list's string keys as named arguments
// (after the positional ones), on every way it is called (#547).
namespace {
    function f($a, $b) { echo "$a $b\n"; }
    function g($a, $b = 'B', $c = 'C') { echo "$a $b $c\n"; }
    function v($a, ...$rest) { echo $a, ' '; var_dump($rest); }
    function r(&$x, $y) { $x .= "+$y"; }
    class C {
        function m($p, $q = 'q') { echo "m $p $q\n"; }
        static function s($p, $q = 'q') { echo "s $p $q\n"; }
        function __call($name, $args) { echo "__call $name "; var_dump($args); }
        static function __callStatic($name, $args) { echo "__callStatic $name "; var_dump($args); }
    }

    function attempt(callable $call) {
        try {
            $call();
        } catch (Throwable $e) {
            echo get_class($e), ': ', $e->getMessage(), "\n";
        }
    }

    function cases(string $how, callable $cufa) {
        echo "-- $how\n";
        $cufa('f', ['b' => 2, 'a' => 1]);
        $cufa('f', [1, 'b' => 2]);
        $cufa('g', ['c' => 'x', 'a' => 1]);
        $cufa('g', [1, 'c' => 'x']);
        attempt(fn() => $cufa('f', ['b' => 2, 1]));
        attempt(fn() => $cufa('f', ['a' => 1, 'z' => 2]));
        attempt(fn() => $cufa('f', [1, 'a' => 2]));
        attempt(fn() => $cufa('g', ['b' => 1]));
        $cufa('v', [1, 2, 'x' => 3, 'y' => 4]);
        $cufa('v', ['a' => 1, 'x' => 3]);
        $n = 'n';
        $cufa('r', ['y' => 'y', 'x' => &$n]);
        echo $n, "\n";
        $cufa('r', ['y' => 'y', 'x' => 'value']);
        $cufa([new C, 'm'], ['q' => 'Q', 'p' => 'P']);
        $cufa('C::s', ['p' => 'P']);
        $cufa([new C, 'missing'], [1, 'k' => 'K']);
        $cufa(['C', 'missingStatic'], ['k' => 'K']);
        $cufa(function ($x, $y = 'Y') { echo "closure $x $y\n"; }, ['y' => 2, 'x' => 1]);
        echo $cufa('str_pad', ['string' => 'a', 'length' => 4, 'pad_type' => STR_PAD_LEFT, 'pad_string' => '-']), "\n";
        echo $cufa('str_replace', ['subject' => 'abc', 'search' => 'b', 'replace' => 'B']), "\n";
        attempt(fn() => $cufa('strlen', ['nope' => 'x']));
        attempt(fn() => $cufa('array_merge', ['x' => [1], 'y' => [2]]));
        echo json_encode($cufa('array_merge', [[1], [2]])), "\n";
    }

    call_user_func_array('f', ['b' => 2, 'a' => 1]);
    // Compiled as PHP compiles it (no call_user_func_array frame).
    cases('global', function ($cb, $list) { return call_user_func_array($cb, $list); });
    // The builtin, called by its name.
    cases('by name', function ($cb, $list) { $f = 'call_user_func_array'; return $f($cb, $list); });
}

namespace N {
    // An unqualified call in a namespace: the builtin.
    \cases('namespaced', function ($cb, $list) { return call_user_func_array($cb, $list); });
}

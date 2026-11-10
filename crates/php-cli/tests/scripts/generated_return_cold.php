<?php
// RETURN (ops.def): to the caller and from the bottom frame of a run of
// the loop (a callback a builtin calls), with its cold cases: an
// undefined variable, a reference, a generator, the main frame. No `@`;
// the expected output is PHP 8.6's.

class D {
    public function __construct(public string $n) {}
    public function __destruct() { echo "~{$this->n};"; }
}

// A temporary, a variable, a literal, an undefined variable (the warning).
function tmp($x) { return $x + 1; }
function cv($x) { $y = $x * 2; return $y; }
function lit() { return 'lit'; }
function undef() { return $nope; }
echo "-- to the caller\n";
var_dump(tmp(1), cv(2), lit(), undef());

// A reference: its value returned, the variable still bound to it.
function ref_var() {
    $a = 1;
    $b = &$a;
    $b = 5;
    return $b;
}
function ref_global() {
    global $g;
    return $g;
}
$g = 'global';
var_dump(ref_var(), ref_global());
// The extra arguments of a frame: its variables are copied out.
function extras($a) {
    $n = func_num_args();
    return $a . $n;
}
var_dump(extras('x', 1, 2));

// Destructors of what the return released, before the caller goes on.
function frees() {
    $d = new D('local');
    $k = 'kept';
    return $k;
}
echo frees(), "\n";
function returns_obj() { return new D('ret'); }
returns_obj();
echo " after\n";

echo "-- from a callback\n";
// array_map / usort call back into PHP: each return leaves that run of the loop.
var_dump(array_map(fn($x) => $x * 10, [1, 2, 3]));
var_dump(array_map(function ($x) { $y = "v$x"; return $y; }, [1, 2]));
var_dump(array_map(function ($x) { return; }, [1]));
var_dump(array_map(function ($x) { if ($x) { return null; } }, [1, 0]));
var_dump(array_map(function ($x) { return $undefined_in_callback; }, [1]));
var_dump(array_filter([1, 0, 2, 3], function ($x) { return $x > 1; }));
$a = [3, 1, 2];
usort($a, function ($x, $y) { return $x <=> $y; });
var_dump($a);
// A destructor due at the callback's return runs before the builtin goes on.
var_dump(array_map(function ($x) { $d = new D("cb$x"); return $x; }, [1, 2]));
var_dump(array_map(function ($x) { $r = &$x; return $r; }, [7]));
// A callback that throws, and one that returns from inside a try/finally.
try {
    array_map(function ($x) { throw new Exception("thrown $x"); }, [1]);
} catch (Exception $e) {
    echo $e->getMessage(), "\n";
}
var_dump(array_map(function ($x) { try { return $x; } finally { echo "finally "; } }, [4]));
// A callback calling another function (the inner return is to the caller).
function inner($x) { return $x + 100; }
var_dump(array_map(function ($x) { return inner($x); }, [1]));
// Nested: a callback inside a callback.
var_dump(array_map(fn($x) => array_sum(array_map(fn($y) => $y * $x, [1, 2])), [1, 2]));

echo "-- generators\n";
function gen() { yield 1; return 'done'; }
$gn = gen();
foreach ($gn as $v) { echo $v, "\n"; }
var_dump($gn->getReturn());
function gen_null() { yield 1; return; }
$gn = gen_null();
foreach ($gn as $v) {}
var_dump($gn->getReturn());

echo "-- a method and a static method\n";
class C {
    public $p = 'prop';
    public function get() { return $this->p; }
    public static function make() { return new static(); }
    public function __toString(): string { return 'C!'; }
}
var_dump((new C)->get(), C::make() instanceof C, (string) new C);
echo "-- main\n";
return;

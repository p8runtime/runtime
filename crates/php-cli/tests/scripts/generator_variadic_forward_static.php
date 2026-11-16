<?php
// A generator function's variadic parameter collects every argument past
// the others (direct, spread and dynamic calls), func_get_args() in it
// sees them all across yields; forward_static_call(_array) pass the
// caller's late static binding on.

function gen(...$a) { yield count($a); yield implode(',', $a); }
function gen2($x, ...$a) { yield count($a); yield implode(',', $a); yield count(func_get_args()); yield func_num_args(); }
function gen3($x = 9, ...$a) { yield "$x:" . implode(',', $a); }

foreach (gen(1, 2, 3) as $v) echo $v, "\n";
foreach (gen2(1, 2, 3) as $v) echo $v, "\n";
$l = [4, 5, 6, 7];
foreach (gen(...$l) as $v) echo $v, "\n";
$f = 'gen2';
foreach ($f(...$l) as $v) echo $v, "\n";
foreach ($f(1) as $v) echo $v, "\n";
echo gen3()->current(), "\n";
echo gen3(1, 2)->current(), "\n";
$c = function (...$a) { yield count($a); yield $a; };
foreach ($c('a', 'b') as $v) echo json_encode($v), "\n";
echo json_encode(iterator_to_array(gen(x: 1, y: 2), false)), "\n";

class A {
    public static function test($x = null) { return static::class . ":" . json_encode(func_get_args()); }
    public static function run() {
        echo forward_static_call(['A', 'test'], 1, 2), "\n";
        echo forward_static_call('A::test', 3), "\n";
        echo forward_static_call_array(['A', 'test'], [4, 5]), "\n";
        echo forward_static_call_array('A::test', ['x' => 6]), "\n";
        echo call_user_func(['A', 'test'], 7), "\n";
        echo forward_static_call('strtoupper', 'up'), "\n";
        echo forward_static_call(fn($v) => static::class . $v, '!'), "\n";
        echo forward_static_call(['C', 'test'], 8), "\n";
    }
}
class B extends A {
    public static function test($x = null) { return 'B::test ' . parent::test($x); }
}
class C {
    public static function test($x = null) { return static::class . ":" . $x; }
}
A::run();
B::run();
echo forward_static_call_array('strtolower', ['DOWN']), "\n";
try {
    forward_static_call('strtoupper', 'x');
} catch (Error $e) {
    echo get_class($e), ": ", $e->getMessage(), "\n";
}
function plain() { return forward_static_call_array(['A', 'test'], [1]); }
echo plain(), "\n";

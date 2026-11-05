<?php
// Partial function application (PHP 8.6): placeholders, named and
// variadic ones, methods, static methods, closures, __call, reflection of
// the closures made, their errors, func_get_args(), references, and the
// pipe operator with a placeholder.

function f($a, $b = 2, ...$rest) {
    return [$a, $b, $rest, func_get_args()];
}

function g($a, $b, $c = 'c') {
    return func_get_args();
}

function inc(&$x, $by = 1) {
    $x += $by;
}

class C {
    public function __construct(public int $base = 10) {}
    public function add($x, $y = 1) { return $this->base + $x + $y; }
    public static function mul($x, $y) { return $x * $y; }
    private function hidden($x) { return "hidden $x"; }
    public function hiddenPartial() { return $this->hidden(?); }
    public function __call($name, $args) { return "$name(" . implode(',', $args) . ")"; }
}

echo "# positional\n";
$p = g(1, ?);
var_dump($p(2));
var_dump($p(2, 3));

echo "# named\n";
$p = g(c: ?, a: 1, b: ?);
var_dump($p('B', 'C'));
echo (string) new ReflectionFunction($p);

echo "# variadic\n";
$p = f(?, ...);
var_dump($p(1));
var_dump($p(1, 2, 3, 4));
$p = g(1, ...);
var_dump($p(2, 3, 4, 5));
echo (string) new ReflectionFunction($p);

echo "# methods\n";
$o = new C(100);
$p = $o->add(?, 5);
var_dump($p(1));
$p = C::mul(?, 3);
var_dump($p(7));
echo (string) new ReflectionFunction($p);
var_dump($o->hiddenPartial()(1));
$p = $o->missing(1, ?);
var_dump($p(2));

echo "# closures\n";
$cl = function ($x, $y) { return "$x/$y"; };
$p = $cl(?, 'y');
var_dump($p('x'));
echo (string) new ReflectionFunction($p);
$p = strlen(...)(?);
var_dump($p('four'));
$p = str_replace('a', ?, ?);
var_dump($p('o', 'banana'));

echo "# references\n";
$n = 1;
$p = inc(?, 5);
$p($n);
var_dump($n);
$p = inc($n, ?);
$p(10);
var_dump($n);

echo "# errors\n";
foreach ([
    fn() => g(?),
    fn() => g(?, ?, ?, ?),
    fn() => g(?, a: 1),
    fn() => g(1, c: ?),
    fn() => g(x: ?),
    fn() => inc(1, ?),
    fn() => array_keys(?, strict: true),
] as $make) {
    try {
        $make();
        echo "made\n";
    } catch (Error $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}
try {
    $p = g(1, ?);
    $p();
} catch (ArgumentCountError $e) {
    echo preg_replace('/ passed in .*? on line \d+/', ' passed', $e->getMessage()), "\n";
}

echo "# pipe\n";
var_dump(5 |> g(1, ?));
var_dump(5 |> g(1, 2, c: ?));
var_dump('banana' |> str_replace('a', 'o', ?));

echo "# debug info\n";
print_r(g(1, ?, c: 3));
echo "\n";

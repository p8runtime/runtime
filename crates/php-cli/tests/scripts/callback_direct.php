<?php
// Callbacks a builtin runs (array_map, array_filter, usort, array_reduce,
// preg_replace_callback) and call_user_func_array of a function's name:
// closures with `use` values (by value and by reference), arrow functions,
// closures bound to $this and static ones, names of functions with
// defaults, too few and too many arguments, by-reference parameters,
// generators, recursion through a builtin, exceptions, and the frames a
// trace shows.

function twice($x) { return $x * 2; }
function keep($x) { return $x > 1; }
function pair($a, $b = 'B') { return "$a-$b"; }
function all_args() { return implode(',', func_get_args()); }
function some_args($a) { return $a . '+' . implode(',', array_slice(func_get_args(), 1)); }
function needs_two($a, $b) { return $a + $b; }
function by_ref(&$x) { $x .= '!'; return $x; }
function gen($x) { yield $x; yield $x + 1; }
function cmp($a, $b) { return $a <=> $b; }
function where() { return implode(' < ', array_map(fn($f) => $f['function'], debug_backtrace())); }
function thrower($x) { if ($x === 3) throw new RuntimeException("three"); return $x; }
function deep($n) { return $n === 0 ? 0 : array_sum(array_map('deep', [$n - 1])) + 1; }
function sum_list(...$xs) { return array_sum($xs); }
function typed(int $x): int { return $x + 1; }

$list = [1, 2, 3, 4];
echo "map name: ", implode(',', array_map('twice', $list)), "\n";
echo "map \\name: ", implode(',', array_map('\twice', $list)), "\n";
echo "filter name: ", implode(',', array_filter($list, 'keep')), "\n";
$k = 10;
echo "map closure use: ", implode(',', array_map(function ($x) use ($k) { return $x + $k; }, $list)), "\n";
echo "map arrow: ", implode(',', array_map(fn($x) => $x * $k, $list)), "\n";
$count = 0;
array_map(function ($x) use (&$count) { $count += $x; }, $list);
echo "use by reference: $count\n";
echo "map static: ", implode(',', array_map(static fn($x) => -$x, $list)), "\n";
echo "map keys too: ", implode(',', array_map(fn($a, $b) => "$a$b", ['a', 'b'], ['x', 'y'])), "\n";
// array_map keeps the keys: a list's, others', a list's with holes.
$m = array_map(fn($x) => $x * 2, [1, 2]);
$m[] = 'next';
var_dump($m);
var_dump(array_map(fn($x) => $x * 2, [3 => 1, 1 => 2]));
var_dump(array_map(fn($x) => $x, ['a' => 1, 'b' => [2]]));
$holes = [1, 2, 3];
unset($holes[1]);
var_dump(array_map(fn($x) => $x, $holes));
var_dump(array_map(fn($x) => $x, []));
echo "map defaults: ", implode(',', array_map('pair', ['p', 'q'])), "\n";
echo "map extra none: ", implode(',', array_map('all_args', [5, 6])), "\n";
echo "filter both: ", implode(',', array_filter(['a' => 1, 'b' => 2], fn($v, $key) => $key === 'b', ARRAY_FILTER_USE_BOTH)), "\n";
echo "reduce: ", array_reduce($list, fn($c, $x) => $c + $x, 100), "\n";
echo "reduce name: ", array_reduce($list, 'needs_two', 0), "\n";
$sorted = [3, 1, 4, 1, 5, 9, 2, 6];
usort($sorted, 'cmp');
echo "usort name: ", implode(',', $sorted), "\n";
usort($sorted, fn($a, $b) => $b <=> $a);
echo "usort closure: ", implode(',', $sorted), "\n";
echo "preg: ", preg_replace_callback('/\d/', fn($m) => $m[0] * 3, 'a1b2c3'), "\n";
echo "variadic: ", array_map('sum_list', [1, 2])[1], "\n";
echo "typed: ", implode(',', array_map('typed', ['4', 5])), "\n";
try {
    array_map('typed', ['x']);
} catch (TypeError $e) {
    echo "typed error: ", $e->getMessage(), "\n";
}

// A by-reference parameter: the general way (its warning).
$words = ['a', 'b'];
echo "by ref: ", implode(',', array_map('by_ref', $words)), "\n";
echo "words: ", implode(',', $words), "\n";
// A generator function: a Generator per element.
foreach (array_map('gen', [10]) as $g) {
    echo "gen: ", implode(',', iterator_to_array($g)), "\n";
}
// Too few arguments for a callback.
try {
    array_map('needs_two', [1]);
} catch (ArgumentCountError $e) {
    echo "count error: ", get_class($e), "\n";
}
// An exception out of a callback ends the builtin.
try {
    array_map('thrower', $list);
} catch (RuntimeException $e) {
    echo "thrown: ", $e->getMessage(), " line ", $e->getLine(), "\n";
    foreach ($e->getTrace() as $f) {
        echo "  ", $f['function'], isset($f['line']) ? " line " . $f['line'] : "", "\n";
    }
}
// Recursion through a builtin.
echo "deep: ", deep(50), "\n";
// The frames under a callback.
echo "where: ", array_map('where', [1])[0], "\n";
echo "where closure: ", array_map(fn($x) => where(), [1])[0], "\n";

class Box
{
    private $items = [];
    private static $made = 0;
    function __construct(private int $step) {}
    function add(...$xs) { foreach ($xs as $x) { $this->items[] = $x; } return $this; }
    function scaled() { return array_map(fn($x) => $x * $this->step, $this->items); }
    function hidden() { return array_map(function ($x) { return $this->secret($x); }, $this->items); }
    private function secret($x) { return "s$x"; }
    static function made() { return array_map(static function ($x) { self::$made += $x; return static::class; }, [1, 2]); }
    function scope() { return array_map(fn() => get_class($this) . '/' . self::class . '/' . static::class, [0]); }
}
class SubBox extends Box {}
$b = (new SubBox(3))->add(1, 2);
echo "this: ", implode(',', $b->scaled()), "\n";
echo "private: ", implode(',', $b->hidden()), "\n";
echo "static: ", implode(',', SubBox::made()), "\n";
echo "scope: ", implode(',', $b->scope()), "\n";

// Closures made from callables: a method's (its $this), a static
// method's, a builtin's, a __call trampoline's, a native method's.
class Magic
{
    function __call($name, $args) { return "$name(" . implode(',', $args) . ")"; }
    static function __callStatic($name, $args) { return "static $name(" . implode(',', $args) . ")"; }
}
echo "method closure: ", implode(',', array_map($b->scaled(...), [0])[0]), "\n";
echo "static closure: ", implode(',', array_map(SubBox::made(...), [0])[0]), "\n";
echo "builtin closure: ", implode(',', array_map(strlen(...), ['ab', 'cde'])), "\n";
echo "trampoline closure: ", implode(' ', array_map((new Magic)->missing(...), [1, 2])), "\n";
echo "trampoline two: ", implode(' ', array_map((new Magic)->pair(...), [1, 2], [3, 4])), "\n";
echo "trampoline sort: ", implode(',', (function () { $l = [3, 1, 2]; usort($l, (new Magic)->cmp(...)); return $l; })()), "\n";
echo "trampoline array: ", implode(' ', array_map([new Magic, 'missing'], [3])), "\n";
echo "static trampoline: ", implode(' ', array_map(Magic::gone(...), [4])), "\n";
echo "fromCallable trampoline: ", implode(' ', array_map(Closure::fromCallable([new Magic, 'other']), [5, 6])), "\n";
$date = new DateTimeImmutable('2020-01-02');
echo "native method closure: ", implode(',', array_map($date->format(...), ['Y', 'm'])), "\n";

// A closure that rebinds itself.
$counter = function () { static $n = 0; return ++$n; };
echo "statics: ", implode(',', array_map($counter, [1, 1, 1])), "\n";

// call_user_func_array of a function's name.
$args = [7, 8];
echo "cufa: ", call_user_func_array('needs_two', $args), "\n";
echo "cufa args kept: ", implode(',', $args), "\n";
echo "cufa defaults: ", call_user_func_array('pair', ['x']), "\n";
echo "cufa extra: ", call_user_func_array('some_args', [1, 2, 3]), "\n";
echo "cufa none: ", call_user_func_array('all_args', []), "\n";
echo "cufa named: ", call_user_func_array('pair', ['b' => 'N', 'a' => 'M']), "\n";
echo "cufa hash list: ", call_user_func_array('needs_two', [1 => 5, 0 => 6]), "\n";
$ref = 'r';
$withRef = [&$ref];
echo "cufa ref list: ", call_user_func_array('by_ref', $withRef), " ", $ref, "\n";
$plain = ['p'];
echo "cufa by-ref param: ", call_user_func_array('by_ref', $plain), " ", $plain[0], "\n";
$x = 1;
$refd = [&$x, 2];
echo "cufa reference value: ", call_user_func_array('needs_two', $refd), "\n";
try {
    call_user_func_array('needs_two', [1]);
} catch (ArgumentCountError $e) {
    echo "cufa count error: ", $e->getMessage(), "\n";
}
echo "cufa many: ", call_user_func_array('all_args', array_fill(0, 300, 1)) === implode(',', array_fill(0, 300, 1)) ? "same" : "differs", "\n";
echo "cufa where: ", call_user_func_array('where', []), "\n";
echo "cufa typed: ", call_user_func_array('typed', ['41']), "\n";
foreach (call_user_func_array('gen', [1]) as $v) {
    echo "cufa gen: $v\n";
}
// Hooks, as WP_Hook::apply_filters runs them.
function add_suffix($value, $extra = '') { return $value . '_s' . $extra; }
function upper($value) { return strtoupper($value); }
$hooks = ['add_suffix', 'upper', 'add_suffix'];
$value = 'v';
foreach ($hooks as $h) {
    $value = call_user_func_array($h, [$value, '1']);
}
echo "hooks: $value\n";

class Dtor { function __construct(public $n) {} function __destruct() { echo "destruct {$this->n}\n"; } }
function first_only($a) { echo "called with $a\n"; return $a; }
echo "cufa list freed: ", call_user_func_array('first_only', ['one']), "\n";
$made = [new Dtor(1)];
call_user_func_array('first_only', ['two', array_pop($made)]);
echo "after\n";

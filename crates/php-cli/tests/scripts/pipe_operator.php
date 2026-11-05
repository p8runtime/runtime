<?php
// PHP 8.5's pipe operator: the value, then the callable called with it.
// First-class callables become plain calls; anything else is called as
// a value. Precedence (below '.', above comparisons), chains, order of
// evaluation, by-reference parameters refusing the value, @prefer-ref,
// assert, and a pipe's result written into (??=).
function inc($x) { echo "inc($x) "; return $x + 1; }
function dbl($x) { echo "dbl($x) "; return $x * 2; }
class M {
    public $k = 3;
    function add($x) { return $x + $this->k; }
    static function neg($x) { return -$x; }
    function __invoke($x) { return "invoked $x"; }
    function chain() { return 5 |> $this->add(...) |> self::neg(...) |> static::neg(...); }
}
var_dump(1 |> inc(...) |> dbl(...));
var_dump(1 |> 'inc' |> 'dbl');
var_dump(4 |> (fn($x) => $x * 10) |> strval(...) |> strrev(...));
var_dump(2 |> [new M, 'add'], 2 |> 'M::neg', 2 |> [M::class, 'neg'], 7 |> new M);
var_dump((new M)->chain());
$m = new M;
var_dump(10 |> $m->add(...), 10 |> M::neg(...), 'abc' |> strtoupper(...), [3, 1, 2] |> count(...));
$f = 'str_repeat';
var_dump('ab' |> (fn($s) => $f($s, 2)));
// Precedence: '.' binds tighter, comparisons and ?? looser.
var_dump('a' . 'b' |> strtoupper(...));
var_dump(5 + 1 |> dbl(...));
var_dump(3 |> dbl(...) < 7, 3 |> dbl(...) == 6, null ?? 4 |> dbl(...));
var_dump((2 |> dbl(...)) . '!', true ? 1 |> inc(...) : 0);
// Order: the value first, then the callable expression.
function make($tag) { echo "make($tag) "; return fn($x) => "$tag:$x"; }
var_dump(inc(0) |> make('m'));
// A by-reference parameter refuses the value; @prefer-ref takes it.
function byref(&$a) { $a++; return $a; }
class R { function m(&$x) { return $x; } static function s(&$y) { return $y; } }
$v = 1;
$r = new R;
foreach (['byref(...)' => fn() => $v |> byref(...), "'byref'" => fn() => $v |> 'byref', '[$o, m]' => fn() => $v |> [new R, 'm'], '$r->m(...)' => fn() => $v |> $r->m(...), 'R::s(...)' => fn() => $v |> R::s(...)] as $what => $call) {
    try { var_dump($call()); } catch (Error $e) { echo "$what: ", get_class($e), ': ', $e->getMessage(), "\n"; }
}
var_dump($v, [3, 1, 2] |> array_multisort(...));
// assert keeps its own source only when called by name.
foreach ([fn() => false |> assert(...), fn() => 0 |> 'assert'] as $call) {
    try { $call(); } catch (AssertionError $e) { var_dump($e->getMessage()); }
}
// Writing into a pipe's result: the pipe runs once.
$calls = 0;
function obj($x) { global $calls; $calls++; return new stdClass; }
var_dump((1 |> obj(...))->p ??= 'default', $calls);
function arr($x) { global $calls; $calls++; return []; }
var_dump((1 |> arr(...))[0] ??= 'first', $calls);
// Generators and exceptions.
function gen($n) { for ($i = 0; $i < $n; $i++) yield $i; }
var_dump(3 |> gen(...) |> iterator_to_array(...));
try { 1 |> (fn($x) => throw new RuntimeException("thrown at $x")); } catch (RuntimeException $e) { echo $e->getMessage(), "\n"; }
try { 1 |> undefined_function(...); } catch (Error $e) { echo $e->getMessage(), "\n"; }
try { 1 |> 'nope'; } catch (Error $e) { echo $e->getMessage(), "\n"; }

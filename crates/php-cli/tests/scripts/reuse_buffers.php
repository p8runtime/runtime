<?php
// Freed array, object-property and argument buffers are reused: what a
// reused buffer held never shows up again, and the values in a freed one
// go when its array goes (destructors at the same moments).
class D {
    function __construct(public $n) {}
    function __destruct() { echo "free {$this->n}\n"; }
}
function make($n, $tag) { $a = []; for ($i = 0; $i < $n; $i++) $a[] = "$tag$i"; return $a; }
function keyed($n, $tag) { $a = []; for ($i = 0; $i < $n; $i++) $a["k$i"] = "$tag$i"; return $a; }
for ($round = 0; $round < 3; $round++) {
    foreach ([1, 7, 8, 9, 15, 16, 17] as $n) {
        $l = make($n, "r$round-");
        $k = keyed($n, "r$round-");
        echo $n, ': ', count($l), ' ', end($l), ' ', count($k), ' ', $k['k' . ($n - 1)], ' ', json_encode(array_slice($l, 0, 2)), "\n";
        unset($l, $k);
    }
    $fresh = [];
    var_dump($fresh, count(make(0, 'x')), [] === keyed(0, 'y'));
}
// Values in a freed buffer go when their array does.
$a = [new D('a1'), 'x', new D('a2')];
echo "before\n";
$a = null;
echo "after\n";
$h = ['p' => new D('h1'), 'q' => 1];
$h = 'gone';
echo "after hash\n";
// A copy written apart, then both freed.
$base = make(6, 'b');
$copy = $base;
$copy[] = 'more';
var_dump(count($base), count($copy), $base[5], $copy[6]);
unset($base);
var_dump(end($copy));
// Growth from 8 to 16 and on.
$g = [];
for ($i = 0; $i < 40; $i++) { $g[] = $i; if ($i === 7 || $i === 8 || $i === 15 || $i === 16) echo count($g), ' '; }
echo array_sum($g), "\n";
$gk = [];
for ($i = 0; $i < 20; $i++) $gk["g$i"] = $i;
echo count($gk), ' ', $gk['g19'], "\n";
// Objects' properties in reused buffers.
class P { public $a = 1; public $b = [2]; public $c; public $d = 'd'; public $e = null; }
for ($i = 0; $i < 3; $i++) {
    $o = new P;
    $o->c = new D("p$i");
    var_dump($o->a, $o->b, $o->d);
    $o2 = clone $o;
    $o2->a = 'changed';
    echo $o->a, ' ', $o2->a, "\n";
    unset($o, $o2);
    echo "objects gone $i\n";
}
// Extra and variadic arguments.
function extra() { return func_get_args(); }
function variadic($first, ...$rest) { return [$first, $rest]; }
for ($i = 0; $i < 3; $i++) {
    var_dump(extra($i, "e$i", [$i]), variadic("v$i", $i, $i + 1));
}
function holds_extra() { return func_get_args(); }
$f = holds_extra(new D('extra arg'), 2);
echo "args kept\n";
var_dump(count($f));
$f = null;
echo "end\n";

<?php
// Temporaries holding objects with destructors: each is destroyed at its
// last use, as PHP frees a TMP operand in the instruction that reads it.
class D {
    public $name;
    function __construct($name) { $this->name = $name; }
    function __destruct() { echo "~{$this->name}\n"; }
    function m() { echo "m {$this->name}\n"; return $this->name; }
    function self() { return $this; }
    function __toString(): string { return "<{$this->name}>"; }
}
function mk($n) { return new D($n); }
function id($x) { return $x; }
function two($a, $b) { echo "two\n"; }
function thrower() { throw new Exception("boom"); }
function pairs() { return [mk('p1'), mk('p2')]; }

echo "-- method on a temporary\n";
mk('a')->m();
echo "after a\n";

echo "-- property of a temporary, then echo\n";
echo mk('b')->name, "\n";
echo "after b\n";

echo "-- concatenation\n";
echo "x " . mk('c')->name . " y\n";
echo "after c\n";

echo "-- __toString of a temporary\n";
echo "s " . mk('d') . "\n";
echo "after d\n";

echo "-- argument\n";
id(mk('e'));
echo "after e\n";

echo "-- nested call result as argument\n";
echo strlen(id(mk('f'))->m()), "\n";
echo "after f\n";

echo "-- chained\n";
$v = mk('g')->self()->self()->name;
echo "got $v\n";

echo "-- exception with a temporary in flight\n";
try {
    two(mk('h'), thrower());
} catch (Exception $e) {
    echo "caught\n";
}
echo "after h\n";

echo "-- exception in a method of a temporary\n";
function t2() { try { mk('i')->self()->m() . thrower(); } catch (Exception $e) { echo "caught i\n"; } echo "end t2\n"; }
t2();

echo "-- break out of foreach over a temporary\n";
foreach (pairs() as $p) {
    echo "it {$p->name}\n";
    break;
}
echo "after break\n";
unset($p);
echo "after unset\n";

echo "-- continue\n";
foreach ([1, 2] as $i) {
    if (mk("k$i")->name === 'k1') {
        echo "continue $i\n";
        continue;
    }
    echo "body $i\n";
}
echo "after continue\n";

echo "-- return from inside foreach over a temporary\n";
function inloop() {
    $local = mk('local');
    foreach (pairs() as $p) {
        return $p->name;
    }
}
echo inloop(), "\n";
echo "after inloop\n";

echo "-- loop condition\n";
$n = 0;
while (mk("w$n")->name !== 'w2') {
    echo "body w$n\n";
    $n++;
}
echo "after while\n";

echo "-- ternary and coalesce\n";
echo (mk('t') ?: null)->name, "\n";
echo (mk('u') ?? null)->name, "\n";
echo "after t u\n";

echo "-- match subject\n";
echo match (mk('mm')->name) { 'mm' => "matched\n", default => "no\n" };
echo "after match\n";

echo "-- switch subject\n";
switch (mk('sw')->name) {
    case 'sw': echo "case sw\n"; break;
}
echo "after switch\n";

echo "-- finally with a temporary return value\n";
function fin() {
    try {
        return mk('r')->name;
    } finally {
        echo "finally\n";
    }
}
echo fin(), "\n";
function fin2() {
    try {
        return mk('r2');
    } finally {
        echo "finally2\n";
    }
}
echo fin2()->name, "\n";
echo "after fin\n";

echo "-- generator with a temporary across yield\n";
function gen() {
    echo two(mk('gy'), yield 1);
    echo "gen end\n";
}
$g = gen();
$g->current();
echo "suspended\n";
unset($g);
echo "after gen destroyed\n";
$g = gen();
$g->current();
$g->send(5);
echo "after gen ran\n";
unset($g);

echo "-- generator destroyed inside foreach over a temporary\n";
function gen2() {
    foreach (pairs() as $p) {
        yield $p->name;
    }
}
foreach (gen2() as $name) {
    echo "y $name\n";
    break;
}
echo "after gen2\n";

echo "-- array literal of temporaries\n";
echo count([mk('l1'), mk('l2')]), "\n";
echo "after literal\n";

echo "-- static call result\n";
class F { static function make($n) { return new D($n); } }
echo F::make('st')->name, "\n";
echo "after static\n";

echo "-- new in an argument\n";
echo id(new D('nw'))->name, "\n";
echo "after new\n";

echo "-- isset and empty on a temporary\n";
var_dump(isset(mk('is')->name));
var_dump(empty(mk('em')->name));
echo "after isset\n";

echo "-- instanceof\n";
var_dump(mk('io') instanceof D);
echo "after instanceof\n";

echo "-- exception out of a foreach over a temporary, caught outside it\n";
function loopthrow() {
    try {
        foreach (pairs() as $p) {
            unset($p);
            thrower();
        }
    } catch (Exception $e) {
        echo "caught loop\n";
    }
    echo "end loopthrow\n";
}
loopthrow();

echo "-- break 2 out of nested foreach over temporaries\n";
foreach (pairs() as $outer) {
    foreach ([mk('in1'), mk('in2')] as $inner) {
        echo "nested {$outer->name} {$inner->name}\n";
        break 2;
    }
}
echo "after break 2\n";
unset($outer, $inner);
echo "after unset 2\n";

echo "-- continue 2 through a switch on a temporary\n";
foreach ([1, 2] as $i) {
    switch (mk("sc$i")->name) {
        case "sc1":
            echo "switch $i\n";
            continue 2;
        default:
            echo "default $i\n";
    }
    echo "after switch $i\n";
}
echo "after continue 2\n";

echo "-- return from a switch on a temporary\n";
function sw() {
    $local = mk('swl');
    switch (mk('sws')->self()) {
        default:
            return "ret";
    }
}
echo sw(), "\n";

echo "-- finally while an exception carries a temporary\n";
function fin3() {
    try {
        two(mk('f3'), thrower());
    } finally {
        echo "finally3\n";
    }
}
try {
    fin3();
} catch (Exception $e) {
    echo "caught fin3\n";
}

echo "-- nested finally around a return of a temporary's method\n";
function fin4() {
    try {
        try {
            return mk('f4')->m();
        } finally {
            echo "inner finally\n";
        }
    } finally {
        echo "outer finally\n";
    }
}
echo fin4(), "\n";

echo "-- a temporary across calls in one expression\n";
echo id(mk('x1'))->name . id(mk('x2'))->name . "\n";
echo "after x\n";

echo "-- a generator yielding temporaries, closed early\n";
function gen3() {
    yield mk('gy1')->name;
    yield mk('gy2')->name;
}
foreach (gen3() as $v) {
    echo "got $v\n";
    break;
}
echo "after gen3\n";

echo "-- return inside try/finally inside foreach over a temporary\n";
function loopfin() {
    foreach (pairs() as $p) {
        try {
            return $p->name;
        } finally {
            echo "finally in loop\n";
        }
    }
}
echo loopfin(), "\n";

echo "-- break inside try/finally inside foreach over a temporary\n";
foreach (pairs() as $p) {
    try {
        break;
    } finally {
        echo "finally before break\n";
    }
}
unset($p);
echo "after break fin\n";

echo "-- return from foreach over a temporary inside try/finally\n";
function finloop() {
    try {
        foreach (pairs() as $p) {
            return $p->name;
        }
    } finally {
        echo "finally around loop\n";
    }
}
echo finloop(), "\n";

echo "-- loop in try in loop, break 2\n";
foreach (pairs() as $o) {
    try {
        foreach ([mk('n1'), mk('n2')] as $i) {
            break 2;
        }
    } finally {
        echo "finally between\n";
    }
}
unset($o, $i);
echo "after nested\n";

echo "-- exception caught inside a foreach over a temporary\n";
foreach (pairs() as $p) {
    try {
        thrower();
    } catch (Exception $e) {
        echo "caught in loop {$p->name}\n";
    }
}
unset($p);
echo "after caught in loop\n";

echo "-- for loop with a temporary in its condition, continue\n";
for ($i = 0; mk("c$i")->name !== 'c2'; $i++) {
    if ($i == 0) {
        continue;
    }
    echo "for body $i\n";
}
echo "after for\n";

echo "-- a temporary live across a call that catches its own exception\n";
function catcher() { try { thrower(); } catch (Exception $e) { return "c"; } }
echo mk('ac')->name . catcher() . "\n";
echo "after ac\n";

echo "-- end\n";

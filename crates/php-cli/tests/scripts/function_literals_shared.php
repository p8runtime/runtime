<?php
// Literal arrays and strings of compiled code are made once and shared by
// every use (Isolate::thaw, Frozen::to_value): each use still sees its own
// copy on write, and a literal array is the array `[k => v, ...]` builds.

function lists() {
    $a = ['x', 'y', 'z'];
    $b = [0 => 'p', 1 => 'q'];
    $c = [1 => 'one', 0 => 'zero'];
    $d = [0 => 'a', 2 => 'c'];
    $e = [];
    return [$a, $b, $c, $d, $e];
}

function written() {
    $a = ['x', 'y', ['in' => [1, 2]]];
    $a[] = 'pushed';
    $a[2]['in'][] = 3;
    $a[0] .= '!';
    return $a;
}

function keys() {
    return ['k' => 1, '1' => 'int one', '01' => 'string 01', '-5' => 'int -5', '-0' => 'string -0', 'k' => 'again', true => 'bool', 2.0 => 'float', null => 'null', 'tail'];
}

function negative() {
    $a = [-3 => 'a', 'b'];
    $a[] = 'c';
    return $a;
}

function first_int_then_string() {
    $a = [5 => 'five', 's' => 'str', 'next'];
    $a[] = 'pushed';
    return $a;
}

function string_literal() {
    $s = 'shared bytes';
    $s[0] = 'S';
    return $s;
}

function by_reference() {
    $a = ['r' => ['deep' => 'orig']];
    $r = &$a['r']['deep'];
    $r = 'changed';
    return $a;
}

function with_static() {
    static $seen = ['first'];
    $seen[] = count($seen);
    return $seen;
}

function defaults($x = ['d' => [1, 2]]) {
    $x['d'][] = 3;
    return $x;
}

const TOP = ['t' => ['u' => 'v'], 'list'];

class Holder {
    const C = ['c' => [1, 2, 3], 'name' => 'holder'];
    public $prop = ['p' => ['q' => 'r']];
    public static $st = ['s' => 1];
}

// Each function twice: the second call must see the literal as compiled,
// not the first call's writes.
for ($i = 0; $i < 2; $i++) {
    echo "-- round $i\n";
    var_dump(lists());
    var_dump(written());
    var_dump(keys());
    var_dump(negative());
    var_dump(first_int_then_string());
    var_dump(string_literal());
    var_dump(by_reference());
    var_dump(with_static());
    var_dump(defaults());
}

$t = TOP;
$t['t']['u'] = 'written';
var_dump(TOP, $t);

$c = Holder::C;
$c['c'][] = 4;
var_dump(Holder::C, $c);

$h1 = new Holder;
$h2 = new Holder;
$h1->prop['p']['q'] = 'h1';
$h1->prop[] = 'added';
var_dump($h1->prop, $h2->prop);
Holder::$st['s']++;
var_dump(Holder::$st, (new Holder)->prop);

// The same literal in two places, one written: the other is untouched.
function same_a() { $x = ['same', 'literal']; $x[0] = 'A'; return $x; }
function same_b() { return ['same', 'literal']; }
var_dump(same_a(), same_b());

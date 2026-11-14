<?php
// `===`, `!==`, `==` and `!=` of every pair of kinds, as a value and as a
// branch, between plain values, through references and against literals;
// strict and loose in_array / array_search, switch and match over the same
// values. Strings come as literals, built at run time (short and long),
// and with their hash known; numbers include -0.0, NAN and INF.

class Pt { public $x = 1; }
enum Suit { case Hearts; case Spades; }

function mk(string $s): string { return substr($s . '!', 0, -1); }
function hashed(string $s): string { $t = mk($s); $seen = [$t => 1]; return isset($seen[$t]) ? $t : ''; }

$warnings = 0;
set_error_handler(function () use (&$warnings) { $warnings++; return true; });

$long = 'a considerably longer string than fits in the head';
$long2 = 'a considerably longer string than fits in the heaD';
$o = new stdClass;
$p = new Pt;
$q = new Pt;
$q->x = 2;
$fn = function () {};
$nan_list = [NAN];

$vals = [
    ['null', null],
    ['false', false],
    ['true', true],
    ['0', 0],
    ['1', 1],
    ['-1', -1],
    ['int max', PHP_INT_MAX],
    ['0.0', 0.0],
    ['-0.0', -0.0],
    ['1.0', 1.0],
    ['1.5', 1.5],
    ['NAN', NAN],
    ['INF', INF],
    ["''", ''],
    ["mk('')", mk('')],
    ["'abc'", 'abc'],
    ["mk('abc')", mk('abc')],
    ["hashed('abc')", hashed('abc')],
    ["'abd'", 'abd'],
    ["hashed('abd')", hashed('abd')],
    ["'ABC'", 'ABC'],
    ["mk('abcd')", mk('abcd')],
    ["':'", ':'],
    ["'9z'", '9z'],
    ["'1'", '1'],
    ["mk('1')", mk('1')],
    ["'1.0'", '1.0'],
    ["'01'", '01'],
    ["' 1'", ' 1'],
    ["'1 '", '1 '],
    ["'1e0'", '1e0'],
    ["'0'", '0'],
    ["'0.0'", '0.0'],
    ["'.5'", '.5'],
    ["mk('0.5')", mk('0.5')],
    ["'1abc'", '1abc'],
    ["mk('1abc')", mk('1abc')],
    ["'1.5'", '1.5'],
    ['long', $long],
    ['mk(long)', mk($long)],
    ['hashed(long)', hashed($long)],
    ['long2', $long2],
    ['hashed(long2)', hashed($long2)],
    ['long . x', $long . 'x'],
    ['[]', []],
    ['[1]', [1]],
    ['[mk 1]', [(int) mk('1')]],
    ["['1']", ['1']],
    ['[1, 2]', [1, 2]],
    ["['a' => 1]", ['a' => 1]],
    ['[[1]]', [[1]]],
    ['[null]', [null]],
];
// Objects, and what they are compared with below: each other, and the
// kinds an object compares with unconverted.
$objs = [
    ['stdClass', $o],
    ['new stdClass', new stdClass],
    ['Pt', $p],
    ['new Pt', new Pt],
    ['Pt x=2', $q],
    ['Hearts', Suit::Hearts],
    ['Spades', Suit::Spades],
    ['closure', $fn],
];
$with_objs = array_merge([
    ['null', null],
    ['false', false],
    ['true', true],
    ["''", ''],
    ["'abc'", 'abc'],
    ["mk('abc')", mk('abc')],
    ['long', $long],
    ['[]', []],
    ['[1]', [1]],
], $objs);

// One character per pair: '#' identical, '=' equal only, '.' neither, '!'
// identical and not equal; 'X' when a negation or a branch disagrees with
// the value.
function pair($a, $b): string {
    $i = $a === $b;
    $ni = $a !== $b;
    $e = $a == $b;
    $ne = $a != $b;
    $ji = 0; if ($a === $b) { $ji = 1; }
    $jni = 0; if ($a !== $b) { $jni = 1; }
    $je = 0; if ($a == $b) { $je = 1; }
    $jne = 0; if ($a != $b) { $jne = 1; }
    $c = $i ? ($e ? '#' : '!') : ($e ? '=' : '.');
    $want = ($i ? 1 : 0) . ($i ? 0 : 1) . ($e ? 1 : 0) . ($e ? 0 : 1) . ($i ? 1 : 0) . ($i ? 0 : 1) . ($e ? 1 : 0) . ($e ? 0 : 1);
    $got = ($i ? 1 : 0) . ($ni ? 1 : 0) . ($e ? 1 : 0) . ($ne ? 1 : 0) . $ji . $jni . $je . $jne;
    return $got === $want ? $c : 'X';
}
function pair_refs(&$a, &$b): string {
    $i = $a === $b;
    $ni = $a !== $b;
    $e = $a == $b;
    $ne = $a != $b;
    $ji = 0; if ($a === $b) { $ji = 1; }
    $jni = 0; if ($a !== $b) { $jni = 1; }
    $je = 0; if ($a == $b) { $je = 1; }
    $jne = 0; if ($a != $b) { $jne = 1; }
    $c = $i ? ($e ? '#' : '!') : ($e ? '=' : '.');
    $want = ($i ? 1 : 0) . ($i ? 0 : 1) . ($e ? 1 : 0) . ($e ? 0 : 1) . ($i ? 1 : 0) . ($i ? 0 : 1) . ($e ? 1 : 0) . ($e ? 0 : 1);
    $got = ($i ? 1 : 0) . ($ni ? 1 : 0) . ($e ? 1 : 0) . ($ne ? 1 : 0) . $ji . $jni . $je . $jne;
    return $got === $want ? $c : 'X';
}
function pair_ref_left(&$a, $b): string {
    $i = $a === $b;
    $e = $a == $b;
    $ji = 0; if ($a !== $b) { $ji = 1; }
    $je = 0; if ($a != $b) { $je = 1; }
    $c = $i ? ($e ? '#' : '!') : ($e ? '=' : '.');
    return ($i ? 0 : 1) . ($e ? 0 : 1) === $ji . $je ? $c : 'X';
}
function pair_ref_right($a, &$b): string {
    $i = $a === $b;
    $e = $a == $b;
    $ji = 0; if ($a !== $b) { $ji = 1; }
    $je = 0; if ($a != $b) { $je = 1; }
    $c = $i ? ($e ? '#' : '!') : ($e ? '=' : '.');
    return ($i ? 0 : 1) . ($e ? 0 : 1) === $ji . $je ? $c : 'X';
}

// Every left against every right: a row of characters per left, then the
// same pairs through references (both sides, either side, one reference
// on both sides), printed only where they differ from the plain row.
function grid(string $title, array $lefts, array $rights): void {
    global $warnings;
    echo "$title (", count($lefts), ' by ', count($rights), "; # identical, = equal only, . neither):\n";
    foreach ($lefts as $i => [$label, $a]) {
        $warnings = 0;
        $row = '';
        foreach ($rights as [, $b]) {
            $row .= pair($a, $b);
        }
        printf("%2d %-14s %s w%d\n", $i, $label, $row, $warnings);
        $both = $left = $right = '';
        foreach ($rights as [, $b]) {
            $x = $a;
            $y = $b;
            $both .= pair_refs($x, $y);
            $left .= pair_ref_left($x, $b);
            $right .= pair_ref_right($a, $y);
        }
        $x = $a;
        $same = pair_refs($x, $x);
        if ($both !== $row || $left !== $row || $right !== $row || $same !== pair($a, $a)) {
            printf("   through references:\n   both  %s\n   left  %s\n   right %s\n   same  %s\n", $both, $left, $right, $same);
        }
    }
}
grid('pairs', $vals, $vals);
grid('objects on the left', $objs, $with_objs);
grid('objects on the right', $with_objs, $objs);

echo "against literals (=== null, null ===, !== null, === false, === true, false !==, === '', === 'abc', 'abc' ===, !== 'abc', === long, === 0, === 1, === 1.0, === [] | == null, == false, == true, == '', == 'abc', 'abc' ==, != 'abc', == long, == 0, == '1', '1' ==, == 1.0, == '1.0', == []):\n";
foreach ($vals as $i => [$label, $v]) {
    $warnings = 0;
    $s = '';
    $s .= $v === null ? 1 : 0;
    $s .= null === $v ? 1 : 0;
    $s .= $v !== null ? 1 : 0;
    $s .= $v === false ? 1 : 0;
    $s .= $v === true ? 1 : 0;
    $s .= false !== $v ? 1 : 0;
    $s .= $v === '' ? 1 : 0;
    $s .= $v === 'abc' ? 1 : 0;
    $s .= 'abc' === $v ? 1 : 0;
    $s .= $v !== 'abc' ? 1 : 0;
    $s .= $v === 'a considerably longer string than fits in the head' ? 1 : 0;
    $s .= $v === 0 ? 1 : 0;
    $s .= $v === 1 ? 1 : 0;
    $s .= $v === 1.0 ? 1 : 0;
    $s .= $v === [] ? 1 : 0;
    $s .= ' ';
    $s .= $v == null ? 1 : 0;
    $s .= $v == false ? 1 : 0;
    $s .= $v == true ? 1 : 0;
    $s .= $v == '' ? 1 : 0;
    $s .= $v == 'abc' ? 1 : 0;
    $s .= 'abc' == $v ? 1 : 0;
    $s .= $v != 'abc' ? 1 : 0;
    $s .= $v == 'a considerably longer string than fits in the head' ? 1 : 0;
    $s .= $v == 0 ? 1 : 0;
    $s .= $v == '1' ? 1 : 0;
    $s .= '1' == $v ? 1 : 0;
    $s .= $v == 1.0 ? 1 : 0;
    $s .= $v == '1.0' ? 1 : 0;
    $s .= $v == [] ? 1 : 0;
    $b = '';
    if ($v === null) { $b .= 'a'; }
    if ($v !== null) { $b .= 'b'; }
    if ($v === '') { $b .= 'c'; }
    if ('' !== $v) { $b .= 'd'; }
    if ($v === 'abc') { $b .= 'e'; }
    if ($v !== 'abc') { $b .= 'f'; }
    if ($v == 'abc') { $b .= 'g'; }
    if ($v != 'abc') { $b .= 'h'; }
    if ($v === false) { $b .= 'i'; }
    if (true !== $v) { $b .= 'j'; }
    if ($v == '1') { $b .= 'k'; }
    if ($v != '1') { $b .= 'l'; }
    if ($v == null) { $b .= 'm'; }
    if ($v != false) { $b .= 'n'; }
    printf("%2d %-14s %s %s w%d\n", $i, $label, $s, $b, $warnings);
}

echo "in_array / array_search (strict, loose; first index found):\n";
$hay = array_column($vals, 1);
foreach ($vals as $i => [$label, $v]) {
    $warnings = 0;
    $strict = array_search($v, $hay, true);
    $loose = array_search($v, $hay);
    $in = (in_array($v, $hay, true) ? 1 : 0) . (in_array($v, $hay) ? 1 : 0) . (in_array($v, [], true) ? 1 : 0);
    $tail = array_slice($hay, $i + 1);
    $later = (in_array($v, $tail, true) ? 1 : 0) . (in_array($v, $tail) ? 1 : 0);
    printf("%2d %-14s %s %s %s %s w%d\n", $i, $label, $strict === false ? '-' : $strict, $loose === false ? '-' : $loose, $in, $later, $warnings);
}

echo "objects in in_array / array_search (strict, loose):\n";
$hay = array_column($with_objs, 1);
foreach ($with_objs as $i => [$label, $v]) {
    $warnings = 0;
    $strict = array_search($v, $hay, true);
    $loose = array_search($v, $hay);
    $tail = array_slice($hay, $i + 1);
    $later = (in_array($v, $tail, true) ? 1 : 0) . (in_array($v, $tail) ? 1 : 0);
    printf("%2d %-14s %s %s %s w%d\n", $i, $label, $strict === false ? '-' : $strict, $loose === false ? '-' : $loose, $later, $warnings);
}
function sw_strings($v): string {
    switch ($v) {
        case 'abc': return 'abc';
        case 'abd': return 'abd';
        case '': return 'empty';
        case '1': return 'one';
        case '0.5': return 'half';
        case 'a considerably longer string than fits in the head': return 'long';
        case ':': return 'colon';
        default: return '-';
    }
}
function sw_mixed($v): string {
    switch ($v) {
        case 1.5: return '1.5';
        case 'abc': return 'abc';
        case -1: return '-1';
        case null: return 'null';
        case [1]: return '[1]';
        case true: return 'true';
        default: return '-';
    }
}
function m_strict($v): string {
    return match ($v) {
        null => 'null',
        false => 'false',
        true => 'true',
        0 => '0',
        1 => '1',
        0.0 => '0.0',
        1.5 => '1.5',
        '' => 'empty',
        'abc' => 'abc',
        '1' => 'one',
        'a considerably longer string than fits in the head' => 'long',
        [] => '[]',
        [1] => '[1]',
        Suit::Hearts => 'hearts',
        default => '-',
    };
}
function m_true($v): string {
    return match (true) {
        $v === null => 'null',
        $v === 'abc', $v === 'abd' => 'ab?',
        $v == '1' => 'loosely one',
        $v !== $v => 'not itself',
        default => '-',
    };
}

echo "switch (strings, mixed) and match (value, true):\n";
foreach ($vals as $i => [$label, $v]) {
    $warnings = 0;
    printf("%2d %-14s %s %s %s %s w%d\n", $i, $label, sw_strings($v), sw_mixed($v), m_strict($v), m_true($v), $warnings);
}
foreach ($objs as $i => [$label, $v]) {
    $warnings = 0;
    printf("%2d %-14s %s %s %s w%d\n", $i, $label, sw_strings($v), m_strict($v), m_true($v), $warnings);
}

// One string, one array, one object against itself and a copy, and what a
// write does to that.
$s = mk('same');
$t = $s;
$r = &$s;
var_dump($s === $s, $s == $s, $s === $t, $s == $t, $r === $t, $r == $s);
$t .= '';
var_dump($s === $t, $s != $t);
$t .= 'x';
var_dump($s === $t, $s == $t, $s !== $t, $s != $t);
$a = [1, 'two', 3.0];
$b = $a;
var_dump($a === $a, $a == $a, $a === $b, $a == $b);
$b[] = 4;
var_dump($a === $b, $a == $b);
$c = $o;
var_dump($o === $c, $o == $c, $o === clone $o, $o == clone $o);
$nan = NAN;
var_dump($nan === $nan, $nan == $nan, $nan !== $nan, $nan != $nan, 0.0 === -0.0, 0.0 == -0.0);
var_dump($nan_list === $nan_list, [NAN] === [NAN]);

// Undefined variables: null to both operators, with a warning each.
$warnings = 0;
var_dump(@$undef === null, @$undef == null, @$undef === '', @$undef == '', @$undef !== $undef2);
echo "warnings: $warnings\n";

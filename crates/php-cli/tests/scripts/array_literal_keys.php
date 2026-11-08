<?php
// Literal string keys (program memory) mixed with computed, numeric-string
// and int keys: the same key found whatever handle names it, PHP's
// normalization and order kept, copies separated on write (#363).

function k(string $s): string { return $s; }

$a = ['id' => 1, 'name' => 'n', '7' => 'seven', '08' => 'eight', '-0' => 'minus zero', 'Id' => 'cased'];
var_dump($a);
var_dump(array_keys($a));

// The same key as a literal, a computed string, a concatenation, an int.
$computed = k('i') . k('d');
var_dump($a['id'], $a[$computed], $a['i' . 'd'], isset($a['id']), isset($a[$computed]), array_key_exists('id', $a));
var_dump($a[7], $a['7'], isset($a['7']), isset($a['08']), isset($a['8']), $a['-0']);
var_dump(isset($a['Id']), isset($a['ID']), $a['Id'] === $a['id']);

// Written through a computed key, read through the literal and back.
$b = [];
$b[$computed] = 'via computed';
$b['id'] .= ' and literal';
$b[k('x')] = 1;
$b['x']++;
$b['y'] ??= 'defaulted';
$b['y'] ??= 'not again';
var_dump($b);

// Order: literal, computed, int; unset then re-added goes last.
$c = ['first' => 1];
$c[k('second')] = 2;
$c[] = 3;
$c['first'] = 'again';
unset($c['first']);
$c['first'] = 'last now';
var_dump($c);
foreach ($c as $key => $v) {
    var_dump(gettype($key), $key);
}

// Copy-on-write: a copy keeps its keys when the original changes, and the
// other way round.
$orig = ['k' => 'v', 'n' => 1];
$copy = $orig;
$copy['k'] = 'changed';
$copy['added'] = true;
unset($orig['n']);
var_dump($orig, $copy);

// A duplicate literal key in one literal: the last value, first position.
var_dump(['a' => 1, 'b' => 2, 'a' => 3]);

// Constant arrays (compiled, shared): the keys are found by literal and
// computed probes, and a copy written to does not touch the constant.
const TABLE = ['alpha' => 'a', 'beta' => 'b', '10' => 'ten'];
class Cfg { const MAP = ['on' => true, 'off' => false]; public static $cache = ['hit' => 0]; }
var_dump(TABLE['alpha'], TABLE[k('beta')], TABLE[10], isset(TABLE['gamma']), Cfg::MAP['on'], Cfg::MAP[k('off')]);
$t = TABLE;
$t['alpha'] = 'changed';
var_dump(TABLE['alpha'], $t['alpha'], Cfg::$cache['hit']++, Cfg::$cache['hit']);
function table(): array { static $s = ['x' => 1]; $s['x']++; return $s; }
table();
var_dump(table());

// Keys that look like ints but are not canonical stay strings; keys given
// as floats, bools and null normalize as PHP does.
$d = [];
$d['123'] = 'int';
$d['0123'] = 'string';
$d['1e3'] = 'string too';
$d[true] = 'one';
$d[1.7] = 'one again';
$d[''] = 'empty';
var_dump($d, array_key_exists(123, $d), array_key_exists('123', $d), isset($d['0123']), isset($d[123]));

// Key functions: flip, search, key compare, in a nested write.
$e = ['one' => 1, 'two' => 2];
var_dump(array_flip($e), array_search(2, $e), array_key_first($e), array_key_last($e), key($e));
$e['nested']['deep'] = 'set';
$e['nested'][k('deep')] .= '!';
var_dump($e['nested']['deep'], array_key_exists('nested', $e));
ksort($a);
var_dump(array_keys($a));
var_dump(json_encode(['id' => 1, k('id') . '2' => 2, '3' => 3]));

// The same literal in two functions is one key everywhere.
function make(): array { return ['shared' => 'made']; }
function read(array $x) { return $x['shared'] ?? 'missing'; }
var_dump(read(make()), read(['shared' => 'inline']), read([k('shared') => 'computed']), read(['other' => 1]));

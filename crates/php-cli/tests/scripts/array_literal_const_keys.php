<?php
// Array literals whose keys are string literals that name no int (set by
// the literal's own string, with no numeric test): order, duplicates,
// int-like strings, lists that become hashes, large literals, keys made
// at run time meeting literal ones, and the keys read back must be PHP's.

class D {
    public function __construct(public string $n) {}
    public function __destruct() { echo "destruct {$this->n}\n"; }
}

function f($x) { return $x * 10; }

$x = 'X';

// Literal keys, literal and computed values, temporaries taken.
var_dump(['a' => 1, 'b' => $x, 'c' => f(2), 'd' => $x . 'y', 'e' => [1], 'f' => null]);

// A key written twice keeps its first place and its last value.
var_dump(['a' => 1, 'b' => 2, 'a' => $x]);
var_dump(['a' => $x, 'b' => 2, 'a' => 3, 'b' => f(1)]);

// Strings that name ints are int keys; other strings stay strings.
var_dump(['5' => 1, '05' => 2, '-0' => 3, '1.5' => 4, '' => 5, '0' => 6, '-3' => 7, ' 2' => 8, '2 ' => 9]);
var_dump(['5' => $x, '05' => $x, '-0' => $x, '' => $x, '0' => $x, '-3' => $x]);
var_dump(['a' => 1, '1' => 2, 'b' => 3, 1 => 4, 'c' => 5, '2' => $x]);

// A list first, then literal string keys: the list becomes a hash.
var_dump([0 => $x, 'a' => 1, 'b' => $x]);
var_dump([$x, 'k' => 2, $x, 'k' => 3]);
$l = [1, 2, 'k' => 3];
$l[] = 4;
var_dump($l);

// More keys than a small table holds; a duplicate past the eighth.
$big = ['k1' => 1, 'k2' => 2, 'k3' => $x, 'k4' => 4, 'k5' => 5, 'k6' => 6, 'k7' => 7, 'k8' => 8, 'k9' => 9, 'k10' => $x, 'k3' => 'again', 'k11' => f(11)];
var_dump($big, count($big), $big['k3'], $big['k10'], isset($big['k12']));

// Keys made at run time meeting literal ones (short and long strings).
$s = 'a' . 'b';
var_dump([$s => 1, 'ab' => 2, 'c' => 3]);
$long = str_repeat('long-key-', 5);
var_dump([$long => 1, 'long-key-long-key-long-key-long-key-long-key-' => 2]);
var_dump([...['a' => 1, 'b' => 2], 'a' => 3, 'c' => 4]);
var_dump(['a' => 1, ...['a' => 2, 'z' => 3], 'b' => 4]);

// The keys read back: lookups, isset, foreach, array functions.
$a = ['name' => 'n', 'value' => $x, 'other' => 3];
var_dump($a['name'], $a['value'], isset($a['other']), array_key_exists('value', $a), isset($a['nope']));
foreach ($a as $k => $v) {
    var_dump($k, $k === 'value', $v);
}
var_dump(array_keys($a), array_flip($a), array_key_first($a), array_key_last($a));
unset($a['value']);
$a['value'] = 'again';
var_dump($a);

// An undefined variable as a value: the warning, then null.
var_dump(['a' => $undef, 'b' => 1]);
var_dump(['a' => 1, 'b' => $undef2, 'b' => 2]);

// A variable bound by reference is read by value.
$r = 1;
$ref = &$r;
$byval = ['a' => $r, 'b' => 2];
$r = 5;
var_dump($byval);

// Built in a loop: a fresh array each time, written after.
$all = [];
for ($i = 0; $i < 3; $i++) {
    $e = ['id' => $i, 'tag' => 't', 'id' => $i * 2];
    $e['tag'] .= $i;
    $all[] = $e;
}
var_dump($all);

// Objects as values go when their array goes; a replaced one at once.
$o = ['first' => new D('1'), 'second' => new D('2'), 'first' => new D('3')];
echo "built\n";
unset($o);
echo "after\n";

// Literals in code compiled at run time.
var_dump(eval('return ["a" => 1, "b" => 2, "a" => 3, "5" => 4, "x" => $x];'));

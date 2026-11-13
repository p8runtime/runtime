<?php
// What one foreach step over an array does with the element, the key and
// the variables it writes: the loop variable written through a reference
// it holds, its old value let go at the step (a destructor runs there),
// a reference element copied out by value, copy-on-write of the array,
// keys as PHP makes them, holes skipped.

class D {
    public function __construct(public string $name) {}
    public function __destruct() { echo "destruct {$this->name}\n"; }
}

// The loop variable and the key variable bound by reference before the
// loop: every step writes through to what they refer to.
$x = 0;
$y = '';
$v = &$x;
$k = &$y;
foreach (['a' => 1, 'b' => 2, 'c' => 3] as $k => $v) {
    echo "step $k=$v x=$x y=$y\n";
}
var_dump($x, $y);
unset($v, $k);

// The old loop value released at the step: its destructor runs before
// the next body, and the last one when the variable goes.
$v = new D('before');
foreach ([new D('one'), new D('two')] as $i => $v) {
    echo "body $i {$v->name}\n";
}
echo "after the loop\n";
unset($v);
echo "unset\n";

// An element that is a reference: by value the loop gets a copy, and a
// write to the loop variable changes neither the element nor its target.
$t = 'target';
$a = ['p' => 'plain', 'r' => &$t, 'n' => [1, 2]];
foreach ($a as $key => $val) {
    if (is_array($val)) {
        $val[] = 3;
    } else {
        $val .= '!';
    }
    echo "$key ", json_encode($val), "\n";
}
var_dump($t, $a['n'], $a['p']);

// Copy-on-write: the loop walks the array as it was when the loop began.
$list = ['x', 'y', 'z'];
foreach ($list as $i => $s) {
    $list[] = "$s$i";
    unset($list[1]);
    echo "$i:$s ";
}
echo "\n", json_encode($list), "\n";

// Keys as PHP makes them: integer-like strings are ints, others stay strings.
foreach (['1' => 'a', '01' => 'b', '-5' => 'c', '1.5' => 'd', true => 'e', 'k' => 'f'] as $key => $val) {
    var_dump($key);
}

// Holes: unset entries are skipped, in a list and in a hashed array, and
// the next free key stays past them.
$h = [10, 20, 30, 40];
unset($h[1], $h[3]);
foreach ($h as $key => $val) {
    echo "$key=>$val ";
}
$h[] = 50;
echo json_encode($h), "\n";
$m = ['a' => 1, 'b' => 2, 'c' => 3];
unset($m['b']);
foreach ($m as $key => $val) {
    echo "$key=>$val ";
}
echo "\n";

// Strings shared between the array and the loop variable: the array keeps
// its values after the loop variable changes, and the count is right
// (the array survives the loop variable going).
$words = [str_repeat('w', 3), str_repeat('q', 2)];
foreach ($words as $word) {
    $word .= '-changed';
}
unset($word);
echo json_encode($words), "\n";

// The same array walked by two loops at once.
$pair = [1, 2];
foreach ($pair as $p) {
    foreach ($pair as $q) {
        echo "$p$q ";
    }
}
echo "\n";

// Values of every kind.
$f = fopen('php://memory', 'r');
foreach ([null, false, 7, 1.5, 'abc', [1], new ArrayObject([]), $f] as $val) {
    echo get_debug_type($val), ' ';
}
echo "\n";

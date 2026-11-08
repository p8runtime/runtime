<?php
// foreach's start and argument sends, run in the loop when plain: arrays
// (empty or not, shared with a variable that changes during the loop),
// references, objects and undefined variables (the warning), the old
// iterator's last holder (a destructor); by-value and by-reference
// arguments to user functions and builtins, and undefined ones.
class D { function __construct(public $n) {} function __destruct() { echo "destruct {$this->n}\n"; } }
class It implements IteratorAggregate { function getIterator(): Iterator { return new ArrayIterator(['x' => 1]); } }
function byval($a, $b = 0) { $a[] = 'added'; return count($a) + $b; }
function byref(&$a) { $a[] = 'ref'; }
function mixed_args($a, &$b, $c) { $b = $a . $c; }
for ($round = 0; $round < 2; $round++) {
    echo "-- $round\n";
    foreach ([] as $v) { echo "never\n"; }
    $arr = [1, 2, 3];
    foreach ($arr as $k => $v) { if ($k == 0) { $arr[] = 4; } echo "$k=$v "; }
    echo count($arr), "\n";
    $r = [5, 6];
    $ref = &$r;
    foreach ($ref as $v) { echo $v, " "; }
    echo "\n";
    foreach (new It as $k => $v) { echo "$k=>$v\n"; }
    foreach ([new D("a$round")] as $o) { echo "in {$o->n}\n"; }
    foreach ([new D("b$round")] as $o) { echo "in {$o->n}\n"; }
    unset($o);
    echo "after\n";
    foreach ($undefined_var ?? [] as $v) {}
    foreach (@$nothing as $v) {}
    $list = ['p', 'q'];
    echo byval($list, 1), " ", count($list), "\n";
    byref($list);
    echo json_encode($list), "\n";
    $x = 'one';
    mixed_args($x, $y, 'two');
    echo $y, "\n";
    $s = [3, 1, 2];
    sort($s);
    echo implode(',', $s), " ", count($s), " ", strlen($x), "\n";
    echo byval($missing ?? []), "\n";
    try { echo byval($undef), "\n"; } catch (Throwable $e) { echo get_class($e), "\n"; }
    $str = 'abc';
    echo strtoupper($str), str_repeat($str, 2), "\n";
}
foreach (5 as $v) {}
echo "end\n";

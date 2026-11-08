<?php
// Overwriting and freeing values shared with other holders: the shared ones
// survive (only a decrement), the last holder's free runs a destructor at
// once, at the write that let go of it.
class D {
    public function __construct(public string $n) {}
    public function __destruct() { echo "destruct {$this->n}\n"; }
}
function consts() { $a = [1, 2, 'x' => 'long string literal that is not short']; $a[] = 3; return $a; }
function consts2() { return ['k' => 'v', 1.5, PHP_INT_MAX]; }
for ($i = 0; $i < 3; $i++) {
    echo "-- $i\n";
    $a = new D("a$i");
    $b = $a;
    $a = null;
    echo "a null\n";
    $b = 1;
    echo "b int\n";
    $arr = [new D("in-array$i")];
    $copy = $arr;
    $arr = 'x';
    echo "arr overwritten\n";
    unset($copy);
    echo "copy unset\n";
    $s = str_repeat('ab', 10 + $i);
    $t = $s;
    $s = 0;
    echo strlen($t), "\n";
    $r = new D("ref$i");
    $ref = &$r;
    $r = 2;
    echo "ref written\n";
    unset($ref);
    $o = new D("ret$i");
    $o = consts();
    echo json_encode($o), json_encode(consts2()), "\n";
    $x = consts();
    $x[0] = 'changed';
    echo json_encode(consts()), "\n";
}
echo "end\n";

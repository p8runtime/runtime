<?php
// The generated handlers (ops.def): each specialisation's straight path
// and its cold cases, against PHP's output.

class D {
    public function __construct(public string $n) {}
    public function __destruct() { echo "~{$this->n};"; }
}

class P {
    public $p = 0;
    public int $t = 0;
    public ?D $d = null;
    function set($v) { $this->p = $v; $this->t = 7; return $this->p; }
    function used($v) { return $this->p = $v; }
}

function dims() {
    $a = [1, 2, 3, 'k' => 'v', '5' => 'five'];
    $k = 'k'; $i = 1; $f = 1.5;
    // FETCH_DIM: R and IS, an int literal, a string literal, a register.
    echo $a[2], $a['k'], $a[$k], $a[$i], $a[5], $a['5'], "\n";
    echo $a['missing'] ?? 'd1', $a[99] ?? 'd2', $a[$k] ?? 'd3', "\n";
    echo @$a['gone'], "|", @$a[42], "|", $a[$f - 0.5], "\n";
    $n = null;
    echo $n['x'] ?? 'null-container', "\n";
    $r = &$a;
    echo $r['k'], $r[0], "\n";
    unset($r);
    // ISSET_DIM
    var_dump(isset($a[0]), isset($a['k']), isset($a[$k]), isset($a['nope']), isset($a[7]));
    // ASSIGN_DIM: USED and UNUSED, literal keys and values.
    $a['k'] = 'w';
    $a[3] = 4;
    $a[$k] = 'x';
    $a[$i] = 20;
    $x = ($a['y'] = 'used');
    $y = ($a[$i] = 21);
    echo $a['k'], $a[3], $a[1], $x, $y, "\n";
    // APPEND_DIM: USED and UNUSED, a literal, the array itself.
    $b = [];
    $b[] = 1;
    $b[] = 'two';
    $z = ($b[] = 3);
    $b[] = $b;
    echo count($b), $z, count($b[3]), "\n";
    $full = [PHP_INT_MAX => 1];
    try {
        $full[] = 2;
    } catch (Error $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
    // An appended object's destructor runs when the array goes.
    $objs = [];
    $objs[] = new D('a');
    $objs[] = new D('b');
    echo "appended:";
    $objs = null;
    echo "\n";
}

function jumps($x, $s) {
    // JUMP_CMP: literals on either side (GT/GE are a literal on the left).
    if ($x < 10) echo "lt,";
    if (10 > $x) echo "gt,";
    if (3 >= $x) echo "ge,"; else echo "!ge,";
    if ($x <= 5) echo "le,";
    if ($x == 5) echo "eq,";
    if ($s === 'a') echo "id,";
    if ($s !== 'b') echo "ni,";
    if (null === $s) echo "null,";
    if ($x != $s) echo "ne,";
    // JUMP_COND: a bool, an int, a string, an array, null.
    foreach ([true, 0, '0', 'x', [], [0], null, 0.0, 2.5] as $c) {
        echo $c ? 'T' : 'F';
    }
    echo "\n";
}

function props() {
    $o = new P;
    echo $o->set(5), $o->p, $o->t, $o->used('u'), "\n";
    $o->p = 1;
    $o->p = 'literal';
    $q = ($o->p = 'used');
    echo $q, $o->p, "\n";
    // A typed property: a plain int inline, a coercion or an error cold.
    $o->t = 3;
    $o->t = "4";
    echo $o->t, "\n";
    try {
        $o->t = "x";
    } catch (TypeError $e) {
        echo get_class($e), "\n";
    }
    // The property's old object goes when it is overwritten.
    $o->d = new D('p1');
    $o->d = new D('p2');
    echo "|";
    $o->d = null;
    echo "\n";
    // Undefined variables and properties: the warnings.
    echo @$undefined, @$o->nope, "\n";
}

dims();
jumps(5, 'a');
jumps(11, 'b');
props();
echo "done\n";

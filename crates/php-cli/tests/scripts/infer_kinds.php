<?php
// What the kinds the compiler proves (p8_bytecode::infer) must not change:
// each specialised opcode next to the case that must keep the generic one.
// Every function runs twice (a call site resolves on its first run).

class D {
    public function __construct(public string $n) {}
    public function __destruct() { echo "destruct {$this->n}\n"; }
}

// Copies (MoveVal, AssignVal): arrays are values, objects handles.
function copies() {
    $a = [1, 2];
    $b = $a;
    $b[] = 3;
    $o = new D("copy");
    $p = $o;
    $p->n = "copied";
    $s = "str";
    $t = $s;
    $t .= "ing";
    echo count($a), count($b), " {$o->n} $s $t\n";
}

// AssignTakeVal over an object: its destructor runs at the store.
function overwrite() {
    $x = new D("first");
    $x = strtoupper("second");
    echo "after the store: $x\n";
}

// AddPlain: an int sum that overflows is a float; strings, arrays,
// null and an undefined operand take the generic path.
function sums($n) {
    $s = 0;
    for ($i = 0; $i < $n; $i++) {
        $s = $s + $i;
    }
    $big = PHP_INT_MAX;
    $t = 0;
    $t = $big + 1;
    $u = 0;
    $u = $big - -1;
    $v = 0;
    $v = "5" + "3";
    $w = 0;
    $w = [1] + [5, 6];
    $x = 0;
    $x = null + 2;
    $y = 0;
    $y = PHP_INT_MIN - 1;
    var_dump($s, $t, $u, $v, $w, $x, $y);
    $z = 0;
    try {
        $z = "abc" + 1;
    } catch (TypeError $e) {
        echo get_class($e), "\n";
    }
    var_dump($z);
    $q = 0;
    $q = $undefined + 1;
    var_dump($q);
}

// An object the sum's destination held: released at the store.
function sum_over_object() {
    $o = new D("sum");
    $o = 1;
    $o = $o + 1;
    echo "o = $o\n";
    $k = 1;
    $q = new D("plus");
    $q = $k + 1;
    echo "q = $q\n";
}

// References: never proven, every store goes through them.
function by_ref(&$p) {
    $q = $p;
    $p = $q + 1;
}
function refs() {
    $a = 1;
    $b = &$a;
    $a = 5;
    echo "b = $b\n";
    $c = 1;
    $f = function () use (&$c) { $c = 7; };
    $f();
    $d = $c;
    echo "d = $d\n";
    $e = 1;
    by_ref($e);
    $g = $e;
    echo "g = $g\n";
    $arr = [1, 2];
    foreach ($arr as &$v) {
        $v = $v * 10;
    }
    unset($v);
    $h = $arr;
    echo "h = ", implode(",", $h), "\n";
    $list = [3, 1, 2];
    sort($list);
    $k = $list;
    echo "k = ", implode(",", $k), "\n";
}
function counter() {
    static $n = 0;
    $n = $n + 1;
    $m = $n;
    return $m;
}
$shared = 1;
function globals() {
    global $shared;
    $shared = $shared + 1;
    $copy = $shared;
    return $copy;
}

// Variables set by name.
function by_name($vars, $name) {
    $a = 1;
    extract($vars);
    $b = $a;
    $c = 1;
    $$name = 9;
    $d = $c;
    echo "b = $b, d = $d\n";
}

// Undefined: the warning, then null.
function undefined($flag) {
    if ($flag) {
        $a = 1;
    }
    $b = $a;
    var_dump($b);
}

// A catch sees what the try did before it threw.
function thrower() { throw new Exception("x"); }
function caught() {
    $a = 1;
    try {
        $a = "changed";
        thrower();
    } catch (Exception $e) {
        $b = $a;
        echo "caught: $b\n";
    }
}

// Return types the kinds prove (Nop) and the ones that convert or throw.
function lt(int $a): bool { return $a < 3; }
function len(string $s): int { return strlen($s); }
function plus(int $a): int { return $a + 1; }
function as_float(float $x): float { return $x; }
function num_float(int|float $x): float { return $x; }
function maybe(?bool $b = null): ?bool { return $b; }
function only_bool(?bool $b = null): bool { return $b; }
function weak(int $a): int { return $a; }
function guard($x): int {
    if (is_int($x)) {
        return $x;
    }
    if (!is_string($x)) {
        return -1;
    }
    return $x;
}
function returns() {
    var_dump(lt(2), lt(5), len("four"), as_float(5), num_float(6), maybe(), weak("12"), weak(7.0));
    var_dump(guard(3), guard(2.5), guard("42"));
    try {
        plus(PHP_INT_MAX);
    } catch (TypeError $e) {
        echo $e->getMessage(), "\n";
    }
    try {
        only_bool();
    } catch (TypeError $e) {
        echo $e->getMessage(), "\n";
    }
    try {
        guard("abc");
    } catch (TypeError $e) {
        echo $e->getMessage(), "\n";
    }
}

for ($round = 0; $round < 2; $round++) {
    echo "== round $round\n";
    copies();
    overwrite();
    sums(5);
    sum_over_object();
    refs();
    echo counter(), " ", globals(), "\n";
    by_name(["a" => 2], "c");
    undefined(false);
    undefined(true);
    caught();
    returns();
}

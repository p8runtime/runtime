<?php
// The generated handlers' (ops.def) cold cases, with no `@`: every
// warning, deprecation and error text compared with PHP 8.6's.

class D {
    public function __construct(public string $n) {}
    public function __destruct() { echo "~{$this->n};"; }
}

class AA implements ArrayAccess {
    public array $d = [];
    public function offsetExists(mixed $k): bool { echo "exists(", var_export($k, true), ")"; return isset($this->d[$k]); }
    public function offsetGet(mixed $k): mixed { echo "get(", var_export($k, true), ")"; return $this->d[$k] ?? null; }
    public function offsetSet(mixed $k, mixed $v): void { echo "set(", var_export($k, true), ")"; if ($k === null) { $this->d[] = $v; } else { $this->d[$k] = $v; } }
    public function offsetUnset(mixed $k): void { echo "unset(", var_export($k, true), ")"; unset($this->d[$k]); }
}

function assign_dim() {
    echo "-- assign_dim\n";
    // REG_REG: a register key and a register value, string and int.
    $a = [];
    $k = 'x'; $v = 1; $i = 4;
    $a[$k] = $v;
    $a[$i] = $v;
    // KINT, the result used.
    $r = ($a[3] = $v);
    $s = ($a[$k] = 'y');
    echo json_encode($a), " $r $s\n";
    // Overwriting an element that is a reference: written through it.
    $x = 1; $y = 2;
    $b = [0, 's' => 0];
    $b[0] = &$x;
    $b['s'] = &$y;
    $b[0] = 5;
    $b['s'] = 6;
    $b[$k] = 7;
    echo "$x $y ", json_encode($b), "\n";
    // A plain copy of it (not itself a reference) keeps the reference
    // elements: written through them too.
    $c = $b;
    $c['s'] = 8;
    $c[0] = 9;
    $sk = 's';
    $c[$sk] = 10;
    echo "$x $y\n";
    // Through a reference-held array: int and string keys.
    $arr = [1, 2];
    $ref = &$arr;
    $ref[1] = 9;
    $ref['n'] = 10;
    $ref[$i] = 11;
    echo json_encode($arr), "\n";
    unset($ref);
    // A string's offset.
    $str = "abc";
    $str[1] = 'X';
    $str[5] = 'Y';
    $str[-1] = 'Z';
    echo "[$str]\n";
    try {
        $str['x'] = 'Q';
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
    // Auto-vivification: null, undefined, false (deprecated).
    $n = null;
    $n['a'] = 1;
    $undef['b'] = 2;
    $f = false;
    $f[0] = 3;
    $f2 = false;
    $f2['k'] = 4;
    echo json_encode([$n, $undef, $f, $f2]), "\n";
    // A scalar: the error.
    $int = 5;
    try {
        $int[0] = 1;
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
    // ArrayAccess, literal and register keys; a numeric string literal
    // reaches offsetSet as the int PHP's compiler made it.
    $o = new AA;
    $o['k'] = 1;
    $o[2] = 2;
    $o['5'] = 3;
    $o['05'] = 4;
    $o[$k] = 5;
    $u = ($o['u'] = 6);
    echo " ", json_encode($o->d), " $u\n";
    // The old element's destructor, on the UNUSED path, before the next
    // statement: string and int keys.
    $c = ['k' => new D('old-s'), 0 => new D('old-i')];
    $c['k'] = 1;
    echo "after-s;";
    $c[0] = 2;
    echo "after-i;";
    $c[$k] = new D('reg');
    $c[$k] = null;
    echo "after-reg\n";
}

function append_dim() {
    echo "-- append_dim\n";
    $b = [1];
    $z = ($b[] = 5);
    $w = ($b[] = 'lit');
    echo json_encode($b), " $z $w\n";
    // A reference variable, both ways.
    $arr = [];
    $r = &$arr;
    $r[] = 1;
    $arr[] = 2;
    echo count($arr), count($r), "\n";
    unset($r);
    // null, undefined, false.
    $n = null;
    $n[] = 1;
    $undef[] = 2;
    $f = false;
    $f[] = 3;
    echo json_encode([$n, $undef, $f]), "\n";
    // A string: the error.
    $s = "abc";
    try {
        $s[] = 'd';
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
    // ArrayAccess: offsetSet(null).
    $o = new AA;
    $o[] = 'p';
    $q = ($o[] = 'q');
    echo " ", json_encode($o->d), " $q\n";
}

function fetch_dim() {
    echo "-- fetch_dim\n";
    $s = "hello";
    $one = '1'; $i = 1; $big = 10; $sk = 'x';
    echo $s[1], $s[-1], $s['1'], $s[$one], $s[$i], "\n";
    echo $s[10], "|", $s[$big], "\n";
    echo $s[1] ?? 'd', $s[10] ?? 'miss', "\n";
    try {
        echo $s['x'];
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
    try {
        echo $s[$sk];
    } catch (Throwable $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
    var_dump(isset($s[1]), isset($s[10]), isset($s['1']), isset($s['x']), isset($s[$sk]), isset($s['1.0']));
    var_dump(empty($s[0]), empty($s[10]));
    // ArrayAccess: read, isset, empty, ??; '5' is the int 5.
    $o = new AA;
    $o->d = ['k' => 'v', 5 => 0, 'z' => ''];
    echo $o['k'], $o['5'], $o[$sk] ?? 'dflt', "\n";
    var_dump(isset($o['k']), isset($o['5']), isset($o['nope']), empty($o['z']), empty($o['k']));
    // An element that is a reference.
    $x = 3;
    $a = ['r' => 0, 0 => 0, 'n' => null];
    $a['r'] = &$x;
    $a[0] = &$x;
    $nul = null;
    $a['n'] = &$nul;
    $rk = 'r';
    echo $a['r'], $a[0], $a[$rk], $a['r'] ?? 'd', "\n";
    var_dump(isset($a['r']), isset($a[0]), isset($a['n']), isset($a[$rk]));
    // empty() of an element.
    $e = ['zero' => 0, 'one' => 1, 'str0' => '0', 'arr' => [], 1 => 'x'];
    var_dump(empty($e['zero']), empty($e['one']), empty($e['str0']), empty($e['arr']), empty($e['missing']), empty($e[1]), empty($e[2]));
    // isset on an object that is not ArrayAccess, a scalar, null.
    $std = new stdClass;
    try {
        var_dump(isset($std['x']));
    } catch (Throwable $t) {
        echo get_class($t), ": ", $t->getMessage(), "\n";
    }
    $int = 5; $null = null;
    var_dump(isset($int[0]), isset($null[0]), isset($undefined[0]));
    // A reference-held array.
    $arr = ['a' => 1, 0 => null, 5 => 'five'];
    $ref = &$arr;
    var_dump(isset($ref['a']), isset($ref[0]), isset($ref[5]), isset($ref['5']), isset($ref['b']));
    echo $ref['a'], $ref[5], $ref['5'], "\n";
    // null and float keys, literal and in a register.
    $m = ['' => 'empty', 1 => 'one'];
    $nk = null; $fk = 1.5; $fi = 1.0;
    var_dump(isset($m[null]), isset($m[$nk]), isset($m[1.5]), isset($m[$fk]), isset($m[$fi]));
    echo $m[$nk], $m[$fk], $m[$fi], "\n";
    echo $m[null], $m[1.5], "\n";
    // Missing keys: the warnings ('7' is the int 7).
    echo $m['nope'], $m['7'], $m[$sk], $m[99], "|\n";
    // A container that is not an array.
    $i5 = 5;
    echo $i5[0], $null['k'], "|\n";
    try {
        echo $std['k'];
    } catch (Throwable $t) {
        echo get_class($t), ": ", $t->getMessage(), "\n";
    }
}

function jumps($n) {
    echo "-- jumps\n";
    // Loop-style jump-if-true at the bottom; register-register compares.
    $i = 0; $lim = $n;
    do { echo $i; $i++; } while ($i < $lim);
    echo ",";
    $i = 0;
    do { echo $i; $i++; } while ($i <= $lim);
    echo ",";
    $i = 0;
    while (!($i === $lim)) { echo $i; $i++; }
    echo ",";
    for ($i = 0, $j = 3; $i < $j; $i++) echo $i;
    echo ",";
    $a = 2; $b = 2; $c = '2';
    if ($a < $b) echo "lt"; else echo "!lt";
    if ($a <= $b) echo "le";
    if ($a === $b) echo "id";
    if ($a === $c) echo "id2"; else echo "!id2";
    if ($a == $c) echo "eq";
    if ('10' < '9') echo "numstr"; else echo "!numstr";
    if ('abc' < 'abd') echo "str";
    echo "\n";
    // Undefined variables: the warnings.
    if ($undef1 < 5) echo "u-lt,";
    if ($undef2) echo "u-t,"; else echo "u-f,";
    if (!$undef3) echo "u-not,";
    while ($undef4 === 1) { echo "never"; }
    if ($undef5 === null) echo "u-id,";
    echo "\n";
    // NaN and floats.
    $nan = NAN; $x = 1.5; $y = 2.5; $z = 0.1 + 0.2;
    if ($nan < 1) echo "nan<"; else echo "!nan<";
    if ($nan == $nan) echo "nan=="; else echo "!nan==";
    if ($nan === $nan) echo "nan==="; else echo "!nan===";
    if ($nan) echo "nan-t"; else echo "nan-f";
    if ($x < $y) echo "f<";
    if ($x <= 1.5) echo "f<=";
    if ($z == 0.3) echo "f=="; else echo "!f==";
    if ($x < 2) echo "fi<";
    if (0.0) echo "0.0-t"; else echo "0.0-f";
    echo "\n";
    // A reference as the condition and operand.
    $cond = true;
    $rc = &$cond;
    if ($rc) echo "ref-t,";
    $cond = 0;
    if ($rc) echo "ref-t2,"; else echo "ref-f,";
    $cnt = 0;
    $rl = &$cnt;
    while ($rl < 3) { $cnt++; }
    echo $cnt;
    if ($rl === 3) echo ",ref-id";
    echo "\n";
}

class T {
    public $p = 0;
    public array $arr = [];
    public int $t = 0;
    public readonly int $ro;
    public int $h = 0 {
        set => $value * 2;
        get => $this->h + 1;
    }
    public function __construct() { $this->ro = 1; }
    public function me() { $x = $this; return $x === $this ? "same" : "other"; }
    public function props() {
        echo $this->p, $this->t, $this->ro, $this->h, "\n";
        $this->p = 'lit';
        $this->t = 3;
        $u = ($this->t = 4);
        $this->h = 3;
        echo $this->p, $this->t, $u, $this->h, "\n";
        // A literal array stays the constant it is: each write separates.
        for ($i = 0; $i < 2; $i++) {
            $this->arr = [1];
            $this->arr[] = 2;
            echo count($this->arr);
        }
        echo "\n";
        // Coercion and type errors, literal and register.
        $this->t = "5";
        echo $this->t, "\n";
        try {
            $this->t = "x";
        } catch (TypeError $e) {
            echo get_class($e), ": ", $e->getMessage(), "\n";
        }
        try {
            $this->ro = 2;
        } catch (Error $e) {
            echo get_class($e), ": ", $e->getMessage(), "\n";
        }
        // Undefined property: read warns, write is a dynamic property.
        echo $this->nope, "|\n";
        $this->dyn = 'd';
        echo $this->dyn, "\n";
    }
}

class M {
    private array $data = [];
    public function __get($n) { echo "get($n)"; return $this->data[$n] ?? 'none'; }
    public function __set($n, $v) { echo "set($n)"; $this->data[$n] = $v; }
    public function run() {
        echo $this->a, "\n";
        $this->a = 'lit';
        $this->b = 5;
        $r = ($this->c = 'used');
        echo $this->a, $this->b, $r, "\n";
    }
}

function nothis() {
    return $this;
}

function this_and_props() {
    echo "-- this, props\n";
    $t = new T;
    echo $t->me(), "\n";
    $t->props();
    try {
        nothis();
    } catch (Error $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
    (new M)->run();
    // `$o->p = K` from outside: USED and UNUSED, a hook, readonly.
    $t->p = 'out';
    $v = ($t->p = 7);
    $t->h = 10;
    echo $t->p, $v, $t->h, "\n";
    try {
        $t->ro = 3;
    } catch (Error $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
    $std = new stdClass;
    $std->a = 'x';
    $w = ($std->b = [1, 2]);
    $w[] = 3;
    echo json_encode($std), count($w), "\n";
}

assign_dim();
append_dim();
fetch_dim();
jumps(3);
this_and_props();
echo "done\n";

<?php
// How a method call's arguments are passed: each as the method found for
// the receiver takes it, by reference or by value.
class RefFirst {
    function m(&$a, $b) { $a .= "+a"; $b .= "+b"; return "RefFirst($a, $b)"; }
}
class RefSecond {
    function m($a, &$b) { $a .= "+a"; $b .= "+b"; return "RefSecond($a, $b)"; }
}
class RefBoth {
    function m(&$a, &$b) { $a .= "+a"; $b .= "+b"; return "RefBoth($a, $b)"; }
}
class RefNone {
    function m($a, $b) { $a .= "+a"; $b .= "+b"; return "RefNone($a, $b)"; }
}

// The same variable twice, once by reference and once by value, in both
// orders.
function twice() {
    foreach ([new RefFirst, new RefSecond, new RefBoth, new RefNone] as $o) {
        $x = "x";
        echo $o->m($x, $x), " x=$x\n";
    }
    // (Each class at its own site.)
    $x = "x"; echo (new RefFirst)->m($x, $x), " x=$x\n";
    $x = "x"; echo (new RefSecond)->m($x, $x), " x=$x\n";
}
twice();

// One call site alternating between classes whose methods differ in which
// parameters are by reference.
function alternating() {
    $objects = [new RefFirst, new RefSecond, new RefFirst, new RefNone, new RefSecond, new RefBoth, new RefFirst];
    foreach ($objects as $o) {
        $a = "a"; $b = "b";
        echo $o->m($a, $b), " a=$a b=$b\n";
    }
    // With an element and a property as arguments (the other send).
    $h = new stdClass;
    foreach ($objects as $o) {
        $arr = ["k" => "a"]; $h->p = "b";
        echo $o->m($arr["k"], $h->p), " a={$arr['k']} b={$h->p}\n";
    }
    // An element that does not exist yet: made by a reference, not by a value.
    foreach ([new RefFirst, new RefNone] as $o) {
        $arr = [];
        echo @$o->m($arr["new"], "b"), " ", json_encode($arr), "\n";
    }
}
alternating();

// The receiver is itself a reference.
function through_reference() {
    $o = new RefFirst;
    $r = &$o;
    $x = "x"; $y = "y";
    echo $r->m($x, $y), " x=$x y=$y\n";
    // Replaced through the other name while the arguments run.
    $x = "x";
    echo $r->m($x, [$o = new RefSecond, "y"][1]), " x=$x ", get_class($r), "\n";
    $list = [new RefSecond];
    $e = &$list[0];
    $x = "x"; $y = "y";
    echo $list[0]->m($x, $y), " x=$x y=$y\n";
    echo $e->m($x, $y), " x=$x y=$y\n";
}
through_reference();
function with_global() {
    global $shared;
    $x = "x"; $y = "y";
    for ($i = 0; $i < 2; $i++) {
        echo $shared->m($x, $y), " x=$x y=$y\n";
    }
}
$shared = new RefBoth;
with_global();
function with_static() {
    static $kept = null;
    $kept ??= new RefSecond;
    $x = "x"; $y = "y";
    echo $kept->m($x, $y), " x=$x y=$y\n";
}
with_static(); with_static();

// A literal, a constant, a call's result and a computed value: to a
// by-value parameter, and beside a by-reference one. (PHP refuses a literal
// where a reference is taken, "could not be passed by reference"; p8 does
// not refuse it for any kind of call yet, so that is not in here.)
function ident($v) { return $v; }
function literals() {
    $x = "x";
    foreach ([new RefNone, new RefSecond, new RefNone] as $o) {
        $y = "y";
        echo $o->m("literal", $y), " y=$y\n";
        echo $o->m(PHP_VERSION_ID > 0, $y), " y=$y\n";
        echo $o->m(ident($x), $y), " y=$y\n";
        echo $o->m($x . "!", $y), " x=$x y=$y\n";
    }
    echo (new RefNone)->m("literal", "b"), " ", (new RefNone)->m(1, 2.5), "\n";
}
literals();

// The object as its own argument; `$this` as an argument.
class Self_ {
    public $tag = "self";
    function take(&$o, $again) { $was = $o; $o = "replaced"; return $was === $this && $again === $this ? "same object" : "different"; }
    function value($o) { $o = "replaced"; return "by value"; }
    function pass_this() { return $this->value($this) . ", " . ($this instanceof Self_ ? "still \$this" : "lost"); }
    function chain(&$x) { $x .= "+"; return $this; }
}
function own_argument() {
    $o = new Self_;
    echo $o->take($o, $o), " ", var_export($o, true), "\n";
    $o = new Self_;
    echo $o->value($o), " ", get_class($o), "\n";
    echo $o->pass_this(), "\n";
    $x = "x";
    $o->chain($x)->chain($x)->chain($x);
    echo "x=$x\n";
}
own_argument();

// More parameters than a site keeps bits for, the late ones by reference;
// a variadic parameter by reference.
class Wide {
    function m($p0, $p1, $p2, $p3, $p4, $p5, $p6, $p7, $p8, $p9, $p10, $p11, $p12, $p13, $p14, $p15, $p16, $p17, $p18, $p19, $p20, $p21, $p22, &$p23, $p24, &$p25, $p26, &$p27) {
        $p22 = "no"; $p23 = "w23"; $p24 = "no"; $p25 = "w25"; $p26 = "no"; $p27 = "w27";
        return "Wide::m";
    }
    function rest($first, &...$rest) { foreach ($rest as $i => &$r) { $r = "r$i"; } return count($rest); }
    function values($first, ...$rest) { foreach ($rest as $i => &$r) { $r = "r$i"; } return count($rest); }
}
function wide() {
    $o = new Wide;
    for ($i = 0; $i < 2; $i++) {
        $v = array_fill(0, 28, "v");
        [$a22, $a23, $a24, $a25, $a26, $a27] = ["v", "v", "v", "v", "v", "v"];
        echo $o->m(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, $a22, $a23, $a24, $a25, $a26, $a27), " $a22 $a23 $a24 $a25 $a26 $a27\n";
        $a = "a"; $b = "b"; $c = "c";
        echo $o->rest($a, $b, $c), " $a $b $c\n";
        $a = "a"; $b = "b"; $c = "c";
        echo $o->values($a, $b, $c), " $a $b $c\n";
        $many = array_fill(0, 30, "v");
        [$m0, $m1, $m2, $m3, $m4, $m5, $m6, $m7, $m8, $m9, $m10, $m11, $m12, $m13, $m14, $m15, $m16, $m17, $m18, $m19, $m20, $m21, $m22, $m23, $m24, $m25, $m26] = $many;
        echo $o->rest($m0, $m1, $m2, $m3, $m4, $m5, $m6, $m7, $m8, $m9, $m10, $m11, $m12, $m13, $m14, $m15, $m16, $m17, $m18, $m19, $m20, $m21, $m22, $m23, $m24, $m25, $m26), " $m0 $m1 $m23 $m24 $m25 $m26\n";
    }
}
wide();

// A builtin's method that takes a reference, and one that does not.
function builtin() {
    $it = new ArrayObject([3, 1, 2]);
    $m = [];
    $r = preg_match('/b/', 'abc', $m);
    $dt = new DateTime('2026-01-02 03:04:05', new DateTimeZone('UTC'));
    $format = "Y-m-d";
    echo $dt->format($format), " $format ", $it->count(), "\n";
    $stmt = new SplObjectStorage;
    $key = new stdClass;
    $stmt->offsetSet($key, "data");
    echo count($stmt), " ", get_class($key), "\n";
}
builtin();

// Too few arguments, and a method that is a generator: not the short way in.
class Odd {
    function needs($a, $b) { return "needs"; }
    function gen(&$x) { $x .= "+gen"; yield 1; yield 2; }
    static function st(&$x) { $x .= "+static"; return isset($this) ? "has this" : "no this"; }
}
function odd() {
    $o = new Odd;
    for ($i = 0; $i < 2; $i++) {
        $x = "x";
        try { echo $o->needs($x), "\n"; } catch (ArgumentCountError $e) { echo $e->getMessage(), "\n"; }
        $g = $o->gen($x);
        echo "before: $x; ";
        foreach ($g as $v) { echo $v, " "; }
        echo "after: $x\n";
        $x = "x";
        echo $o->st($x), " $x\n";
    }
}
odd();

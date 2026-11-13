<?php
// Property reads through a site's cache, each site run several times so
// the cached path answers: every kind of value copied out (and the copy
// independent of the slot), a reference in the slot, unset and
// uninitialized slots, isset, private and inherited properties from each
// scope, one site seeing several classes, `$o->$name`, and when a copy
// read out keeps an object (and its destructor) alive.

class D {
    public function __construct(public string $n) {}
    public function __destruct() { echo "destruct {$this->n}\n"; }
}

class K {
    public $null = null;
    public $false = false;
    public $int = 42;
    public $float = 1.5;
    public $lit = "literal";
    public $str;
    public $arr = [1, [2, 3]];
    public $obj;
    public $later;
    public int $typed;
    public readonly int $ro;
    private $secret = "k-secret";

    public function __construct() {
        $this->str = str_repeat("ab", 3);
        $this->obj = new D("in K");
        $this->ro = 7;
    }

    public function secret() { return $this->secret; }
}

class Sub extends K {
    private $secret = "sub-secret";
    public function mine() { return $this->secret; }
}

class Magic {
    public $set = "set";
    public $gone = "x";
    public function __get($n) { return "__get($n)"; }
}

function show($label, $v) {
    echo $label, ": ";
    var_dump($v);
}

$k = new K;
for ($round = 0; $round < 2; $round++) {
    show("null", $k->null);
    show("false", $k->false);
    show("int", $k->int);
    show("float", $k->float);
    show("lit", $k->lit);
    show("str", $k->str);
    show("arr", $k->arr);
    show("obj", $k->obj->n);
    show("ro", $k->ro);
}

// The copy is the value, not the slot: changing it leaves the property.
for ($i = 0; $i < 3; $i++) {
    $a = $k->arr;
    $a[1][] = $i;
    $s = $k->str;
    $s .= "!";
    echo count($k->arr[1]), " ", $k->str, " ", count($a[1]), " ", $s, "\n";
}

// A reference in the slot: reads see the referenced value.
$r = &$k->int;
for ($i = 0; $i < 3; $i++) {
    $r = $i * 10;
    show("through ref", $k->int);
    $copy = $k->int;
    $copy++;
    echo "copy $copy, still ", $k->int, "\n";
}
unset($r);

// isset of null, false, set, unset and uninitialized properties.
for ($i = 0; $i < 2; $i++) {
    var_dump(isset($k->null), isset($k->false), isset($k->int), isset($k->later), isset($k->typed));
}

// An unset slot: the warning, every time; then __get for a class with it.
unset($k->float);
for ($i = 0; $i < 2; $i++) {
    show("unset", @$k->float);
    echo error_get_last()["message"] ?? "-", "\n";
}
$m = new Magic;
for ($i = 0; $i < 3; $i++) {
    if ($i == 1) {
        unset($m->gone);
    }
    show("magic", $m->gone);
    show("magic set", $m->set);
}

// An uninitialized typed property: the Error, every time.
for ($i = 0; $i < 2; $i++) {
    try {
        $k->typed;
    } catch (Error $e) {
        echo get_class($e), ": ", $e->getMessage(), "\n";
    }
}

// Private properties: each scope sees its own; one site, two classes.
$sub = new Sub;
foreach ([$k, $sub, $k, $sub] as $o) {
    echo get_class($o), " ", $o->secret(), " ", $o->int, " ", $o->lit, "\n";
}
echo $sub->mine(), "\n";
for ($i = 0; $i < 2; $i++) {
    try {
        echo $sub->secret;
    } catch (Error $e) {
        echo $e->getMessage(), "\n";
    }
}

// `$o->$name`, cached path and not.
foreach (["int", "lit", "arr", "nope", "int"] as $name) {
    show("dyn $name", @$k->$name);
}

// A copy read out keeps the object alive after the property lets go.
$holder = new K;
for ($i = 0; $i < 2; $i++) {
    $kept = $holder->obj;
    echo "kept ", $kept->n, "\n";
}
$holder->obj = null;
echo "property cleared\n";
unset($kept);
echo "copy dropped\n";
$holder = null;
echo "end\n";

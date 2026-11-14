<?php
// Property writes, each form of the write instruction (`$o->p = v` and
// `$this->p = v`; a variable, a literal or a temporary value; the result
// used or not) against each kind of property and receiver, every site run
// several times so its cache entry answers: few properties and many, one
// site seeing several classes and several scopes, the slot holding a
// reference, the value a reference or undefined, the old value's
// destructor, typed, readonly, set-visibility, magic, hooked, dynamic and
// lazy properties, and what is not an object at all.

class D {
    public function __construct(public string $n) {}
    public function __destruct() { echo "destruct {$this->n}\n"; }
}

function show(string $label, mixed $v): void {
    echo $label, ': ', is_object($v) ? get_class($v) : json_encode($v), "\n";
}

function attempt(string $label, callable $f): void {
    try {
        $f();
        echo $label, ": ok\n";
    } catch (Throwable $e) {
        echo $label, ': ', get_class($e), ': ', $e->getMessage(), "\n";
    }
}

// Every form, from a register and from $this.
class W {
    public $a = 0;
    public $b = 'b';
    public function forms($v, $i) {
        $this->a = $v;
        $this->b = 'lit';
        $this->a = $v . "-$i";
        $r = ($this->b = $v);
        $s = ($this->a = "t$i");
        return "$r $s";
    }
}
$w = new W;
for ($i = 0; $i < 4; $i++) {
    $v = "v$i";
    $w->a = $v;
    echo $w->a, ' ';
    $w->b = 7;
    echo $w->b, ' ';
    $w->a = $v . 'x';
    echo $w->a, ' ';
    $r = ($w->b = $v);
    $s = ($w->a = $i * 2);
    echo $r, ' ', $s, ' ', $w->forms($v, $i), ' ', $w->a, ' ', $w->b, "\n";
}

// One to six declared properties (kept in the object, and past four in a
// buffer of their own): the last of each, through one site that sees
// every class, and the first.
class P1 { public $a = 'a1'; }
class P2 extends P1 { public $b = 'b2'; }
class P3 extends P2 { public $c = 'c3'; }
class P4 extends P3 { public $d = 'd4'; }
class P5 extends P4 { public $e = 'e5'; }
class P6 extends P5 { public $f = 'f6'; }
class Q6 { public $f = 'qf'; public $e = 'qe'; public $d = 'qd'; public $c = 'qc'; public $b = 'qb'; public $a = 'qa'; }
function set_a(object $o, $v) { $o->a = $v; }
function set_d(object $o, $v) { $o->d = $v; }
$objects = [new P1, new P6, new P4, new Q6, new P5, new P2, new P6, new Q6, new P3, new P1, new P4];
foreach ($objects as $k => $o) {
    set_a($o, "A$k");
    if (property_exists($o, 'd')) {
        set_d($o, "D$k");
    }
}
foreach ($objects as $o) {
    echo get_class($o), ' ', json_encode(get_object_vars($o)), "\n";
}

// The old value: an object with a destructor goes at the write, before
// the next statement; an array, a string, a value kept elsewhere too.
class Holder { public $held; public function put($v) { $this->held = $v; } }
$h = new Holder;
for ($i = 0; $i < 3; $i++) {
    $h->held = new D("by register $i");
    echo "after register write $i\n";
}
for ($i = 0; $i < 3; $i++) {
    $h->put(new D("by this $i"));
    echo "after this write $i\n";
}
$keep = new D('kept');
$h->held = $keep;
$h->held = [1, 2, 3];
echo "kept still here\n";
$h->held = str_repeat('x', 5);
$h->held = $h->held . 'y';
echo $h->held, "\n";
unset($keep);
echo "after unset kept\n";
$used = ($h->held = new D('used result'));
$used2 = ($h->held = 1);
echo "after used write\n";
unset($used);

// The slot holds a reference: the write goes through it.
class R { public $p = 1; public function set($v) { $this->p = $v; } }
$o = new R;
$ref = &$o->p;
for ($i = 0; $i < 3; $i++) {
    $o->p = $i;
    echo "ref $ref ";
    $o->set($i + 10);
    echo "ref $ref\n";
}
unset($ref);
$o->p = 'after unset ref';
echo $o->p, "\n";

// The value is a reference (by-reference variable, global): its value is
// written, not the reference.
function by_ref_value(R $o) {
    $x = 1;
    $y = &$x;
    for ($i = 0; $i < 3; $i++) {
        $o->p = $y;
        $x = 'changed';
        echo json_encode($o->p), ' ';
        $x = $i;
    }
    echo "\n";
}
by_ref_value(new R);
$g = 'global value';
function from_global(R $o) {
    global $g;
    for ($i = 0; $i < 3; $i++) {
        $o->p = $g;
        $g = "global $i";
    }
    return $o->p;
}
echo from_global(new R), "\n";

// An undefined variable: the warning, and null written.
function undefined_value(R $o, int $n) {
    for ($i = 0; $i < $n; $i++) {
        if ($i == 2) {
            $o->p = $undef;
        } else {
            $o->p = $i;
        }
        show("undefined $i", $o->p);
    }
}
undefined_value(new R, 4);

// Typed properties: coercion, TypeError, uninitialized ones.
class T {
    public int $i = 0;
    public ?string $s = null;
    public float $f = 0.0;
    public array $arr = [];
    public int $uninit;
    public mixed $m = 'm';
    public function seti($v) { $this->i = $v; }
}
function set_i(T $t, $v) { $t->i = $v; }
function set_s(T $t, $v) { $t->s = $v; }
function set_f(T $t, $v) { $t->f = $v; }
function set_m(T $t, $v) { $t->m = $v; }
$t = new T;
foreach ([1, 2, '3', 4.0, true, 'nope', 5, null, [1], 6] as $v) {
    attempt('int ' . json_encode($v), function () use ($t, $v) { set_i($t, $v); });
    attempt('this int ' . json_encode($v), function () use ($t, $v) { $t->seti($v); });
    attempt('string ' . json_encode($v), function () use ($t, $v) { set_s($t, $v); });
    attempt('float ' . json_encode($v), function () use ($t, $v) { set_f($t, $v); });
    set_m($t, $v);
    echo json_encode([$t->i, $t->s, $t->f, $t->m]), "\n";
}
for ($i = 0; $i < 3; $i++) {
    $u = new T;
    attempt("uninit read $i", function () use ($u) { return $u->uninit; });
    $u->uninit = "4$i";
    var_dump($u->uninit);
}

// Readonly: the first write from the class, then refused, each time at
// the same site; an untyped-looking one (mixed).
class RO {
    public readonly mixed $m;
    public readonly int $n;
    public function set($m, $n) { $this->m = $m; $this->n = $n; }
}
for ($i = 0; $i < 3; $i++) {
    $ro = new RO;
    $ro->set("m$i", $i);
    attempt("readonly again $i", function () use ($ro, $i) { $ro->set("again $i", 9); });
    attempt("readonly outside $i", function () use ($ro) { $ro->m = 'outside'; });
    echo $ro->m, ' ', $ro->n, "\n";
}

// Set visibility: written from the class, refused from outside.
class AV {
    public private(set) mixed $p = 'start';
    public protected(set) mixed $q = 'q';
    public function set($v) { $this->p = $v; $this->q = $v; }
}
class AVChild extends AV {
    public function child($v) { $this->q = "child $v"; }
    public function childP($v) { $this->p = $v; }
}
function set_av_p(AV $o, $v) { $o->p = $v; }
for ($i = 0; $i < 3; $i++) {
    $av = $i == 1 ? new AVChild : new AV;
    $av->set("inside $i");
    attempt("set visibility outside $i", function () use ($av, $i) { set_av_p($av, "outside $i"); });
    if ($av instanceof AVChild) {
        $av->child($i);
        attempt("set visibility child $i", function () use ($av, $i) { $av->childP($i); });
    }
    echo $av->p, ' ', $av->q, "\n";
}

// __set and its kin: a set declared property is written; an unset one
// goes to __set, at a site that wrote it before.
class Magic {
    public $p = 'declared';
    public function __set($k, $v) { echo "__set $k = ", json_encode($v), "\n"; }
    public function __get($k) { return "__get $k"; }
}
function set_p(object $o, $v) { $o->p = $v; }
$m = new Magic;
for ($i = 0; $i < 4; $i++) {
    set_p($m, "write $i");
    echo $m->p, "\n";
    if ($i == 1) {
        unset($m->p);
    }
}
// Without them, an unset declared property is written again.
class NoMagic { public $p = 'declared'; }
$nm = new NoMagic;
for ($i = 0; $i < 4; $i++) {
    set_p($nm, "write $i");
    echo $nm->p, "\n";
    unset($nm->p);
    echo isset($nm->p) ? 'set' : 'unset', "\n";
}
// The same site, magic and plain objects in turn.
foreach ([$nm, $m, new NoMagic, new Magic, $nm] as $k => $o) {
    set_p($o, "turn $k");
    echo get_class($o), ' ', $o->p, "\n";
}

// Hooks: the set hook runs every time.
class Hooked {
    public $plain = 'plain';
    public string $hooked = 'h' {
        set(string $v) { echo "hook set $v\n"; $this->hooked = strtoupper($v); }
    }
}
$hk = new Hooked;
for ($i = 0; $i < 3; $i++) {
    $hk->plain = "plain $i";
    $hk->hooked = "hooked $i";
    echo $hk->plain, ' ', $hk->hooked, "\n";
}

// Dynamic properties: the deprecation once per write, stdClass none.
class Dyn { public $declared = 1; }
function set_dyn(object $o, $v) { $o->extra = $v; }
for ($i = 0; $i < 3; $i++) {
    $d = new Dyn;
    set_dyn($d, "extra $i");
    set_dyn($d, "extra again $i");
    $s = new stdClass;
    set_dyn($s, "std $i");
    echo $d->extra, ' ', $s->extra, "\n";
}

// Visibility: each scope its own private property through one site; a
// closure bound to each scope in turn; refused from outside.
class Base {
    private $secret = 'base secret';
    public function setSecret($v) { $this->secret = $v; }
    public function getSecret() { return $this->secret; }
}
class Child extends Base {
    private $secret = 'child secret';
    public function setMine($v) { $this->secret = $v; }
    public function getMine() { return $this->secret; }
}
$base = new Base;
$child = new Child;
$writer = function ($v) { $this->secret = $v; };
$loose = function ($o, $v) { $o->secret = $v; };
foreach ([[$base, Base::class], [$child, Child::class], [$child, Base::class], [$base, Base::class], [$child, Child::class]] as $k => [$o, $scope]) {
    Closure::bind($writer, $o, $scope)("bound $k");
    Closure::bind($loose, null, $scope)($o, "loose $k");
    echo $scope, ' ', get_class($o), ' ', $o->getSecret(), ' / ', $o instanceof Child ? $o->getMine() : '-', "\n";
}
foreach ([$base, $child, $child, $base] as $k => $o) {
    $o->setSecret("method $k");
    if ($o instanceof Child) {
        $o->setMine("mine $k");
    }
    attempt("outside $k", function () use ($o, $k) { $o->secret = "outside $k"; });
    echo get_class($o), ' ', $o->getSecret(), ' / ', $o instanceof Child ? $o->getMine() : '-', "\n";
}

// A lazy object written at a site that has seen its class: initialized
// by the write, once, then written.
class L {
    public $x = 'default x';
    public $y = 'default y';
    public function __construct() { echo "(L constructed) "; $this->x = 'made x'; $this->y = 'made y'; }
    public function setX($v) { $this->x = $v; }
}
function write_x(L $l, $v) { $l->x = $v; }
$plain = new L;
write_x($plain, 'plain 1');
write_x($plain, 'plain 2');
echo $plain->x, "\n";
$class = new ReflectionClass(L::class);
$ghost = $class->newLazyGhost(function (L $l) { $l->__construct(); });
$proxy = $class->newLazyProxy(function () { return new L; });
$inside = $class->newLazyGhost(function (L $l) { $l->__construct(); });
write_x($ghost, 'ghost 1');
echo "ghost: ", $ghost->x, ' ', $ghost->y, "\n";
write_x($ghost, 'ghost 2');
echo "ghost: ", $ghost->x, "\n";
write_x($proxy, 'proxy 1');
echo "proxy: ", $proxy->x, ' ', $proxy->y, "\n";
$inside->setX('inside 1');
echo "inside: ", $inside->x, ' ', $inside->y, "\n";
write_x($plain, 'plain 3');
echo "plain again: ", $plain->x, "\n";

// The receiver held by a reference.
function by_ref_receiver(R &$o, $v) { $o->p = $v; }
$rr = new R;
for ($i = 0; $i < 3; $i++) {
    by_ref_receiver($rr, "through reference $i");
    echo $rr->p, "\n";
}

// Not an object: the error, each time.
function prop_to($x) { $x->p = 1; }
foreach ([new R, null, 5, 'text', [1], new R] as $x) {
    attempt(gettype($x), function () use ($x) { prop_to($x); });
}
echo "end\n";

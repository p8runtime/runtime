<?php
// Property reads, each form of the read instruction (`$o->p`, `$this->p`,
// `$o->p ?? d`, `$this->p ?? d`, `$o?->p`) against each kind of property
// and receiver, every site run several times so its cache entry answers:
// few properties and many (kept in the object, and in a buffer of their
// own), one site seeing several classes and several scopes, a receiver
// held by a reference, declared, dynamic, typed, readonly, hooked, magic
// and lazy properties, unset and uninitialized ones, and what is not an
// object at all.

class D {
    public function __construct(public string $n) {}
    public function __destruct() { echo "destruct {$this->n}\n"; }
}

function show(string $label, mixed $v): void {
    echo $label, ': ', is_object($v) ? get_class($v) : json_encode($v), "\n";
}

// One to six declared properties: the last of each, the first, one past.
class P1 { public $a = 'a1'; }
class P2 extends P1 { public $b = 'b2'; }
class P3 extends P2 { public $c = 'c3'; }
class P4 extends P3 { public $d = 'd4'; }
class P5 extends P4 { public $e = 'e5'; }
class P6 extends P5 { public $f = 'f6'; }
function letters(object $o): string {
    return $o->a . ($o->b ?? '-') . ($o->c ?? '-') . ($o->d ?? '-') . ($o->e ?? '-') . ($o->f ?? '-');
}
foreach ([new P1, new P6, new P4, new P5, new P2, new P3, new P6, new P1, new P4] as $o) {
    echo get_class($o), ' ', letters($o), "\n";
}
// The same site, an object of few properties after one of many: the
// slot the entry names is not there.
function sixth(object $o) { return @$o->f; }
foreach ([new P6, new P6, new P3, new P6, new P4, new P1] as $o) {
    show('sixth of ' . get_class($o), sixth($o));
}
$n = new P6;
$sum = '';
for ($i = 0; $i < 4; $i++) {
    $sum .= $n->a . $n->d . $n->e . $n->f;
    $n->e = "e$i";
    $n->a = "a$i";
}
echo $sum, "\n";

// Every kind of value, from a register and from $this, plain and quiet.
class V {
    public $null = null;
    public $true = true;
    public $int = 7;
    public $float = 0.5;
    public $lit = 'lit';
    public $str;
    public $arr = [1, 2];
    public $obj;
    public function __construct() {
        $this->str = str_repeat('s', 3);
        $this->obj = new D('held by V');
    }
    public function own(): array {
        return [$this->null, $this->true, $this->int, $this->float, $this->lit, $this->str, $this->arr, get_class($this->obj)];
    }
    public function ownQuiet(): array {
        return [$this->null ?? 'd', $this->true ?? 'd', $this->int ?? 'd', $this->float ?? 'd', $this->lit ?? 'd', $this->str ?? 'd', $this->arr ?? 'd', $this->nope ?? 'd'];
    }
}
$v = new V;
for ($i = 0; $i < 3; $i++) {
    echo json_encode([$v->null, $v->true, $v->int, $v->float, $v->lit, $v->str, $v->arr, get_class($v->obj)]), "\n";
    echo json_encode([$v->null ?? 'd', $v->true ?? 'd', $v->int ?? 'd', $v->float ?? 'd', $v->lit ?? 'd', $v->str ?? 'd', $v->arr ?? 'd', $v->nope ?? 'd']), "\n";
    echo json_encode($v->own()), "\n";
    echo json_encode($v->ownQuiet()), "\n";
    echo json_encode([$v?->int, $v?->str, $v?->arr]), "\n";
}

// What is read is a copy: the property keeps its own.
for ($i = 0; $i < 3; $i++) {
    $a = $v->arr;
    $a[] = $i;
    $s = $v->str;
    $s .= $i;
    $n = $v->int;
    $n += $i;
    echo json_encode([$a, $s, $n, $v->arr, $v->str, $v->int]), "\n";
}

// The read's result replaces the last handle of an object: its
// destructor runs there.
for ($i = 0; $i < 2; $i++) {
    $x = new D("replaced $i");
    $x = $v->int;
    echo "after the read $i: $x\n";
}

// A reference in the slot, and a receiver held by a reference.
$r = &$v->int;
for ($i = 0; $i < 3; $i++) {
    $r = $i * 100;
    echo $v->int, ' ', $v->int ?? 'd', ' ', json_encode($v->own()[2]), "\n";
}
unset($r);
function through_global(): string {
    global $v, $nothing, $number;
    $out = '';
    for ($i = 0; $i < 3; $i++) {
        $out .= $v->lit . ',' . ($v->nope ?? 'd') . ',' . ($nothing->lit ?? 'none') . ',' . ($number->lit ?? 'no object') . ';';
    }
    return $out;
}
$nothing = null;
$number = 5;
echo through_global(), "\n";
function through_param(&$o): string {
    $out = '';
    for ($i = 0; $i < 3; $i++) {
        $out .= $o->lit . $o->int . ($o->later ?? '?');
        $o->later = $i;
    }
    return $out;
}
echo through_param($v), "\n";
$bound = &$v;
for ($i = 0; $i < 2; $i++) {
    echo $bound->str, $bound?->lit, "\n";
}
unset($bound);

// A property holding a reference, read where a reference handed on
// would show: sent to a function that changes the variable, appended and
// stored under a key before the variable changes. The value it had.
class Rf {
    public $p = 'declared';
    private $s = 'private';
    function bind(&$to): void { $this->s = &$to; }
    function sent() { return rf_then_change($this->s); }
    function stored(): string {
        $b = [];
        $b[] = $this->s;
        $b['k'] = $this->s;
        $GLOBALS['shared'] = 'changed after';
        return $b[0] . '/' . $b['k'];
    }
}
function rf_then_change($v) {
    $GLOBALS['shared'] = 'changed inside';
    return $v;
}
function rf_bump($v) { $v++; return $v; }
$shared = 'first';
$rf = new Rf;
$rf->p = &$shared;
$rf->bind($shared);
for ($i = 0; $i < 3; $i++) {
    $shared = "first $i";
    echo rf_then_change($rf->p), ' | ', $shared, "\n";
    $shared = "first $i";
    echo $rf->sent(), ' | ', $shared, "\n";
    $shared = $i;
    echo rf_bump($rf->p), ' | ', $shared, "\n";
    $b = [];
    $b[] = $rf->p;
    $b['k'] = $rf->p;
    $shared = 'changed after';
    echo $b[0], '/', $b['k'], ' | ', $shared, "\n";
    $shared = "first $i";
    echo $rf->stored(), ' | ', $shared, "\n";
}
unset($rf);

// Visibility: each scope its own private property through one site; a
// protected one from inside, from a subclass, from a static method, from
// a closure bound to each scope in turn; refused from outside each time.
class Base {
    private $secret = 'base secret';
    protected $shared = 'base shared';
    public $open = 'base open';
    public function secret() { return $this->secret; }
    public function all() { return "$this->secret|$this->shared|$this->open"; }
    public static function peek(Base $o) { return $o->secret . '/' . $o->shared; }
}
class Child extends Base {
    private $secret = 'child secret';
    public function mine() { return $this->secret . '|' . $this->shared; }
    public static function peekChild(Child $o) { return $o->secret . '/' . ($o->shared ?? 'd'); }
}
class Other {
    private $secret = 'other secret';
}
$base = new Base;
$child = new Child;
foreach ([$base, $child, $child, $base] as $o) {
    echo get_class($o), ': ', $o->secret(), ' ; ', $o->all(), ' ; ', Base::peek($o), ' ; ', Child::peekChild($child), "\n";
}
echo $child->mine(), "\n";
$reader = function () { return $this->secret; };
$loose = function ($o) { return $o->secret ?? 'hidden'; };
foreach ([[$base, Base::class], [$child, Child::class], [$child, Base::class], [new Other, Other::class], [$base, Base::class]] as [$o, $scope]) {
    echo $scope, ' sees ', Closure::bind($reader, $o, $scope)(), ' / ', Closure::bind($loose, null, $scope)($o), "\n";
}
foreach (['secret', 'shared', 'open', 'secret'] as $name) {
    foreach ([$base, $child] as $o) {
        try {
            echo match ($name) {
                'secret' => $o->secret,
                'shared' => $o->shared,
                'open' => $o->open,
            }, "\n";
        } catch (Error $e) {
            echo get_class($e), ': ', $e->getMessage(), "\n";
        }
    }
}
var_dump(isset($base->secret), isset($child->shared), isset($base->open), $child?->open);

// Typed and readonly properties; an uninitialized one is an Error each
// time, and null to a quiet read.
class T {
    public int $count = 1;
    public ?D $maybe = null;
    public string $late;
    public readonly array $fixed;
    public function __construct() { $this->fixed = ['f']; }
    public function late() { return $this->late; }
    public function lateQuiet() { return $this->late ?? 'not yet'; }
}
$t = new T;
for ($i = 0; $i < 3; $i++) {
    try {
        echo $t->count, json_encode($t->maybe), json_encode($t->fixed), $t->lateQuiet(), ' ', $t->late ?? 'unset', ' ';
        echo $i == 2 ? $t->late() : $t->late, "\n";
    } catch (Error $e) {
        echo get_class($e), ': ', $e->getMessage(), "\n";
    }
    if ($i == 1) {
        $t->late = 'now';
        $t->count++;
    }
}

// Unset and undefined: the warning every time, null to a quiet read, and
// a property set again read from the same site.
set_error_handler(function ($no, $message) {
    echo "[$no] $message\n";
    return true;
});
class U {
    public $here = 'here';
    public $gone = 'gone';
    public function gone() { return $this->gone; }
    public function goneQuiet() { return $this->gone ?? 'quiet'; }
}
$u = new U;
for ($i = 0; $i < 5; $i++) {
    if ($i == 1) {
        unset($u->gone);
    }
    if ($i == 3) {
        $u->gone = 'back';
    }
    show("gone $i", $u->gone);
    show("gone $i from inside", $u->gone());
    show("gone $i quiet", $u->gone ?? 'quiet');
    show("gone $i quiet from inside", $u->goneQuiet());
    show("never $i", $u->never);
    show("here $i", $u->here);
}

// Dynamic properties, of stdClass and of a class that allows them.
#[AllowDynamicProperties]
class Bag { public $declared = 'declared'; }
$std = new stdClass;
$bag = new Bag;
for ($i = 0; $i < 4; $i++) {
    show("std $i", $std->extra ?? 'none');
    show("bag $i", $bag->extra ?? 'none');
    show("bag declared $i", $bag->declared);
    if ($i == 0) {
        $std->extra = 'std extra';
        $bag->extra = ['bag extra'];
    }
    if ($i == 2) {
        unset($std->extra, $bag->extra);
    }
}
show('std undefined', $std->extra);
foreach ([$bag, $std, new P1, $bag] as $o) {
    show('one site: ' . get_class($o), $o->declared ?? $o->a ?? 'neither');
}

// Magic: __get for what is not there or not visible, never for what is.
class M {
    public $real = 'real';
    private $hidden = 'hidden';
    public $unset = 'to be unset';
    public function __get($name) { return "__get($name)"; }
    public function __isset($name) { echo "__isset($name) "; return $name != 'no'; }
    public function inside() { return $this->hidden . ',' . $this->real . ',' . $this->missing; }
}
$m = new M;
for ($i = 0; $i < 3; $i++) {
    if ($i == 1) {
        unset($m->unset);
    }
    echo $m->real, ' ', $m->hidden, ' ', $m->unset, ' ', $m->missing, ' ', $m->inside(), "\n";
    echo $m->real ?? 'd', ' ', $m->hidden ?? 'd', ' ', $m->unset ?? 'd', ' ', $m->no ?? 'd', "\n";
}

// Hooks: the get hook runs each time; the backing value from inside it.
class H {
    public $plain = 'plain';
    public $hooked = 'backing' {
        get { echo '(get) '; return strtoupper($this->hooked); }
    }
    public $virtual { get => $this->plain . '!'; }
}
$h = new H;
for ($i = 0; $i < 3; $i++) {
    echo $h->plain, ' ', $h->hooked, ' ', $h->virtual, ' ', $h->hooked ?? 'd', "\n";
    $h->plain = "plain $i";
}

// A lazy object read at a site that has seen its class: initialized by
// the read, once.
class L {
    public $x = 'default x';
    public $y = 'default y';
    public function __construct() { echo "(L constructed) "; $this->x = 'made x'; }
    public function x() { return $this->x; }
}
function read_x(L $l) { return $l->x; }
$plain = new L;
echo read_x($plain), ' ', read_x($plain), "\n";
$class = new ReflectionClass(L::class);
$ghost = $class->newLazyGhost(function (L $l) { $l->__construct(); });
$proxy = $class->newLazyProxy(function () { return new L; });
$inside = $class->newLazyGhost(function (L $l) { $l->__construct(); });
echo "ghost: ", read_x($ghost), ' ', read_x($ghost), ' ', $ghost->y, "\n";
echo "proxy: ", read_x($proxy), ' ', read_x($proxy), ' ', $proxy->y, "\n";
echo "inside: ", $inside->x(), ' ', $inside->x(), "\n";
echo "plain again: ", read_x($plain), ' ', $plain->x(), "\n";

// Not an object: the warning, each time; nothing said to a quiet read.
function prop_of($x) { return $x->p; }
function quiet_prop_of($x) { return $x->p ?? 'd'; }
function safe_prop_of($x) { return $x?->p; }
class WithP { public $p = 'p of an object'; }
foreach ([new WithP, null, 5, 'text', [1], 1.5, new WithP] as $x) {
    show(gettype($x), prop_of($x));
    show(gettype($x) . ' quiet', quiet_prop_of($x));
    if ($x === null || is_object($x)) {
        show(gettype($x) . ' safe', safe_prop_of($x));
    }
}
// (Its line number is this file's, so only the message.)
$unbound = function () { return $this->p; };
try {
    $unbound();
} catch (Error $e) {
    echo get_class($e), ': ', $e->getMessage(), "\n";
}
$v = null;
echo "end\n";
